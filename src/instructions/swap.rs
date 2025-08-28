use crate::{AmmState, Config};
use constant_product_curve::{ConstantProduct, LiquidityPair};
use pinocchio::account_info::AccountInfo;
use pinocchio::instruction::{Seed, Signer};
use pinocchio::program_error::ProgramError;
use pinocchio::pubkey::find_program_address;
use pinocchio::sysvars::clock::Clock;
use pinocchio::sysvars::Sysvar;
use pinocchio::ProgramResult;
use pinocchio_token::instructions::Transfer;
use pinocchio_token::state::TokenAccount;

pub struct SwapAccounts<'a> {
    pub user: &'a AccountInfo,
    pub user_x_ata: &'a AccountInfo,
    pub user_y_ata: &'a AccountInfo,
    pub vault_x: &'a AccountInfo,
    pub vault_y: &'a AccountInfo,
    pub config: &'a AccountInfo,
    pub token_program: &'a AccountInfo,
}

impl<'a> TryFrom<&'a [AccountInfo]> for SwapAccounts<'a> {
    type Error = ProgramError;

    fn try_from(accounts: &'a [AccountInfo]) -> Result<Self, Self::Error> {
        let [user, user_x_ata, user_y_ata, vault_x, vault_y, config, token_program] = accounts
        else {
            return Err(ProgramError::NotEnoughAccountKeys);
        };

        // Basic Accounts Checks
        // SignerAccount::check(initializer)?;
        // MintInterface::check(mint_lp)?;

        // check initializer is signer
        // but it will be checked downstream maybe?

        Ok(Self {
            user,
            user_x_ata,
            user_y_ata,
            vault_x,
            vault_y,
            config,
            token_program,
        })
    }
}

#[repr(C)]
pub struct SwapInstructionData {
    pub is_x: bool,
    pub amount: u64,
    pub min: u64,
    pub expiration: i64,
}

impl<'a> TryFrom<&'a [u8]> for SwapInstructionData {
    type Error = ProgramError;

    fn try_from(data: &'a [u8]) -> Result<Self, Self::Error> {
        if data.len().ne(&(size_of::<u64>() * 3 + size_of::<bool>())) {
            return Err(ProgramError::InvalidInstructionData);
        }

        let is_x = data[0] != 0;
        let amount = u64::from_le_bytes(data[1..9].try_into().unwrap());

        if amount == 0 {
            return Err(ProgramError::InvalidInstructionData);
        }

        let min = u64::from_le_bytes(data[9..17].try_into().unwrap());
        let expiration = i64::from_le_bytes(data[17..25].try_into().unwrap());

        // Check signature expiration
        let now = Clock::get()?.unix_timestamp;
        if now > expiration {
            return Err(ProgramError::InvalidInstructionData);
        }

        Ok(Self {
            is_x,
            amount,
            min,
            expiration,
        })
    }
}

pub struct Swap<'a> {
    pub accounts: SwapAccounts<'a>,
    pub instruction_data: SwapInstructionData,
}

impl<'a> TryFrom<(&'a [u8], &'a [AccountInfo])> for Swap<'a> {
    type Error = ProgramError;

    fn try_from((data, accounts): (&'a [u8], &'a [AccountInfo])) -> Result<Self, Self::Error> {
        let accounts = SwapAccounts::try_from(accounts)?;
        let instruction_data = SwapInstructionData::try_from(data)?;

        // Return the initialized struct
        Ok(Self {
            accounts,
            instruction_data,
        })
    }
}

impl<'a> Swap<'a> {
    pub const DISCRIMINATOR: &'a u8 = &3;

    pub fn process(&mut self) -> ProgramResult {
        let config = Config::load(self.accounts.config)?;

        if config.state().ne(&(AmmState::Initialized as u8)) {
            return Err(ProgramError::InvalidAccountData);
        }

        // Check if the vault_x is valid
        let (vault_x, _) = find_program_address(
            &[
                self.accounts.config.key(),
                self.accounts.token_program.key(),
                config.mint_x(),
            ],
            &pinocchio_associated_token_account::ID,
        );

        if vault_x.ne(self.accounts.vault_x.key()) {
            return Err(ProgramError::InvalidAccountData);
        }

        // Check if the vault_y is valid
        let (vault_y, _) = find_program_address(
            &[
                self.accounts.config.key(),
                self.accounts.token_program.key(),
                config.mint_y(),
            ],
            &pinocchio_associated_token_account::ID,
        );

        if vault_y.ne(self.accounts.vault_y.key()) {
            return Err(ProgramError::InvalidAccountData);
        }

        // Deserialize the token accounts
        let vault_x = unsafe { TokenAccount::from_account_info_unchecked(self.accounts.vault_x)? };
        let vault_y = unsafe { TokenAccount::from_account_info_unchecked(self.accounts.vault_y)? };

        // Swap Calculations
        let mut curve = ConstantProduct::init(
            vault_x.amount(),
            vault_y.amount(),
            vault_x.amount(),
            config.fee(),
            None,
        )
        .map_err(|_| ProgramError::Custom(1))?;

        let p = match self.instruction_data.is_x {
            true => LiquidityPair::X,
            false => LiquidityPair::Y,
        };

        let swap_result = curve
            .swap(p, self.instruction_data.amount, self.instruction_data.min)
            .map_err(|_| ProgramError::Custom(1))?;

        // Check for correct values
        if swap_result.deposit == 0 || swap_result.withdraw == 0 {
            return Err(ProgramError::InvalidArgument);
        }

        let seed_binding = config.seed().to_le_bytes();
        let config_bump_binding = config.config_bump();
        let config_seeds = [
            Seed::from(b"config"),
            Seed::from(&seed_binding),
            Seed::from(config.mint_x()),
            Seed::from(config.mint_y()),
            Seed::from(&config_bump_binding),
        ];

        let signer_seeds = [Signer::from(&config_seeds)];

        if self.instruction_data.is_x {
            Transfer {
                amount: swap_result.deposit,
                authority: self.accounts.user,
                from: self.accounts.user_x_ata,
                to: self.accounts.vault_x,
            }
            .invoke()?;

            Transfer {
                amount: swap_result.withdraw,
                authority: self.accounts.config,
                from: self.accounts.vault_y,
                to: self.accounts.user_y_ata,
            }
            .invoke_signed(&signer_seeds)?;
        } else {
            Transfer {
                amount: swap_result.deposit,
                authority: self.accounts.user,
                from: self.accounts.user_y_ata,
                to: self.accounts.vault_y,
            }
            .invoke()?;

            Transfer {
                amount: swap_result.withdraw,
                authority: self.accounts.config,
                from: self.accounts.vault_x,
                to: self.accounts.user_x_ata,
            }
            .invoke_signed(&signer_seeds)?;
        }

        Ok(())
    }
}
