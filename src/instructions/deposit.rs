use crate::{AmmState, Config};
use constant_product_curve::ConstantProduct;
use pinocchio::account_info::AccountInfo;
use pinocchio::program_error::ProgramError;
use pinocchio::pubkey::find_program_address;
use pinocchio::sysvars::clock::Clock;
use pinocchio::sysvars::Sysvar;
use pinocchio::ProgramResult;
use pinocchio_token::instructions::MintTo;
use pinocchio_token::instructions::Transfer;
use pinocchio_token::state::{Mint, TokenAccount};
use pinocchio::instruction::{Seed, Signer};

pub struct DepositAccounts<'a> {
    pub user: &'a AccountInfo,
    pub mint_lp: &'a AccountInfo,
    pub vault_x: &'a AccountInfo,
    pub vault_y: &'a AccountInfo,
    pub user_x_ata: &'a AccountInfo,
    pub user_y_ata: &'a AccountInfo,
    pub user_lp_ata: &'a AccountInfo,
    pub config: &'a AccountInfo,
    pub token_program: &'a AccountInfo,
}

impl<'a> TryFrom<&'a [AccountInfo]> for DepositAccounts<'a> {
    type Error = ProgramError;

    fn try_from(accounts: &'a [AccountInfo]) -> Result<Self, Self::Error> {
        let [user, mint_lp, vault_x, vault_y, user_x_ata, user_y_ata, user_lp_ata, config, token_program] =
            accounts
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
            mint_lp,
            vault_x,
            vault_y,
            user_x_ata,
            user_y_ata,
            user_lp_ata,
            config,
            token_program,
        })
    }
}

#[repr(C)]
pub struct DepositInstructionData {
    /// Amount the user wishes to receive
    pub amount: u64,
    pub max_x: u64,
    pub max_y: u64,
    pub expiration: i64,
}

impl<'a> TryFrom<&'a [u8]> for DepositInstructionData {
    type Error = ProgramError;

    fn try_from(data: &'a [u8]) -> Result<Self, Self::Error> {
        if data.len() != size_of::<u64>() * 4 {
            return Err(ProgramError::InvalidInstructionData);
        }

        let amount = u64::from_le_bytes(data[0..8].try_into().unwrap());

        if amount == 0 {
            return Err(ProgramError::InvalidInstructionData);
        }

        let max_x = u64::from_le_bytes(data[8..16].try_into().unwrap());
        let max_y = u64::from_le_bytes(data[16..24].try_into().unwrap());
        let expiration = i64::from_le_bytes(data[24..32].try_into().unwrap());

        // Check signature expiration
        let now = Clock::get()?.unix_timestamp;
        if now > expiration {
            return Err(ProgramError::InvalidInstructionData);
        }

        Ok(Self {
            amount,
            max_x,
            max_y,
            expiration,
        })
    }
}

pub struct Deposit<'a> {
    pub accounts: DepositAccounts<'a>,
    pub instruction_data: DepositInstructionData,
}

impl<'a> TryFrom<(&'a [u8], &'a [AccountInfo])> for Deposit<'a> {
    type Error = ProgramError;

    fn try_from((data, accounts): (&'a [u8], &'a [AccountInfo])) -> Result<Self, Self::Error> {
        let accounts = DepositAccounts::try_from(accounts)?;
        let instruction_data = DepositInstructionData::try_from(data)?;

        // Return the initialized struct
        Ok(Self {
            accounts,
            instruction_data,
        })
    }
}

impl<'a> Deposit<'a> {
    pub const DISCRIMINATOR: &'a u8 = &1;

    pub fn process(&mut self) -> ProgramResult {
        let config = Config::load(self.accounts.config)?;

        if config.state().ne(&(AmmState::Initialized as u8)) {
            return Err(ProgramError::InvalidAccountData);
        }

        // Check if the vault_x ATA is valid
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

        // Check if the vault_y ATA is valid
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
        let mint_lp = unsafe { Mint::from_account_info_unchecked(self.accounts.mint_lp)? };
        let vault_x = unsafe { TokenAccount::from_account_info_unchecked(self.accounts.vault_x)? };
        let vault_y = unsafe { TokenAccount::from_account_info_unchecked(self.accounts.vault_y)? };

        // Grab the amounts to deposit
        let (x, y) = match mint_lp.supply() == 0 && vault_x.amount() == 0 && vault_y.amount() == 0 {
            true => (self.instruction_data.max_x, self.instruction_data.max_y),
            false => {
                let amounts = ConstantProduct::xy_deposit_amounts_from_l(
                    vault_x.amount(),
                    vault_y.amount(),
                    mint_lp.supply(),
                    self.instruction_data.amount,
                    6,
                )
                .map_err(|_| ProgramError::InvalidArgument)?;

                (amounts.x, amounts.y)
            }
        };

        // Check for slippage
        if !(x <= self.instruction_data.max_x && y <= self.instruction_data.max_y) {
            return Err(ProgramError::InvalidArgument);
        }

        // Transfer the amounts from the token accounts of the user to the vaults
        Transfer {
            from: self.accounts.user_x_ata,
            to: self.accounts.vault_x,
            authority: self.accounts.user,
            amount: x,
        }
        .invoke()?;

        Transfer {
            from: self.accounts.user_y_ata,
            to: self.accounts.vault_y,
            authority: self.accounts.user,
            amount: y,
        }
        .invoke()?;

        // and mint the appropriate amount of LP tokens to the user token account
        let seed_binding = config.seed().to_le_bytes();
        let config_bump_binding = config.config_bump();
        let config_seeds = [
            Seed::from(b"config"),
            Seed::from(&seed_binding),
            Seed::from(config.mint_x()),
            Seed::from(config.mint_y()),
            Seed::from(&config_bump_binding),
        ];

        let signer = [Signer::from(&config_seeds)];

        MintTo {
        // minting happens to the User LP ATA
            account: self.accounts.user_lp_ata,
            amount: self.instruction_data.amount,
            mint: self.accounts.mint_lp,
        // the authority is still the pool
            mint_authority: self.accounts.config,
        }
        .invoke_signed(&signer)?;

        Ok(())
    }
}
