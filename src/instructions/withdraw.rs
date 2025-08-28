use crate::{AmmState, Config};
use constant_product_curve::ConstantProduct;
use pinocchio::account_info::AccountInfo;
use pinocchio::instruction::{Seed, Signer};
use pinocchio::msg;
use pinocchio::program_error::ProgramError;
use pinocchio::pubkey::find_program_address;
use pinocchio::sysvars::clock::Clock;
use pinocchio::sysvars::Sysvar;
use pinocchio::ProgramResult;
use pinocchio_token::instructions::{Burn, Transfer};
use pinocchio_token::state::{Mint, TokenAccount};

pub struct WithdrawAccounts<'a> {
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

impl<'a> TryFrom<&'a [AccountInfo]> for WithdrawAccounts<'a> {
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

pub struct WithdrawInstructionData {
    pub amount: u64,
    pub min_x: u64,
    pub min_y: u64,
    pub expiration: i64,
}

impl<'a> TryFrom<&'a [u8]> for WithdrawInstructionData {
    type Error = ProgramError;

    fn try_from(data: &'a [u8]) -> Result<Self, Self::Error> {
        if data.len().ne(&(size_of::<u64>() * 4)) {
            return Err(ProgramError::InvalidInstructionData);
        }

        let amount = u64::from_le_bytes(data[0..8].try_into().unwrap());

        if amount == 0 {
            return Err(ProgramError::InvalidInstructionData);
        }

        let min_x = u64::from_le_bytes(data[8..16].try_into().unwrap());
        let min_y = u64::from_le_bytes(data[16..24].try_into().unwrap());
        let expiration = i64::from_le_bytes(data[24..32].try_into().unwrap());

        // Check signature expiration
        let now = Clock::get()?.unix_timestamp;
        if now > expiration {
            return Err(ProgramError::InvalidInstructionData);
        }

        Ok(Self {
            amount,
            min_x,
            min_y,
            expiration,
        })
    }
}

pub struct Withdraw<'a> {
    pub accounts: WithdrawAccounts<'a>,
    pub instruction_data: WithdrawInstructionData,
}

impl<'a> TryFrom<(&'a [u8], &'a [AccountInfo])> for Withdraw<'a> {
    type Error = ProgramError;

    fn try_from((data, accounts): (&'a [u8], &'a [AccountInfo])) -> Result<Self, Self::Error> {
        let accounts = WithdrawAccounts::try_from(accounts)?;
        let instruction_data = WithdrawInstructionData::try_from(data)?;

        // Return the initialized struct
        Ok(Self {
            accounts,
            instruction_data,
        })
    }
}

impl<'a> Withdraw<'a> {
    pub const DISCRIMINATOR: &'a u8 = &2;

    pub fn process(&mut self) -> ProgramResult {
        let config = Config::load(self.accounts.config)?;

        if config.state().eq(&(AmmState::Disabled as u8)) {
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

        let (x, y) = match mint_lp.supply() == self.instruction_data.amount {
            true => (vault_x.amount(), vault_y.amount()),
            false => {
                let amounts = ConstantProduct::xy_withdraw_amounts_from_l(
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

        msg!("Here 5!");

        // Check for slippage
        if !(x >= self.instruction_data.min_x && y >= self.instruction_data.min_y) {
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

        let signer = [Signer::from(&config_seeds)];

        // Transfer the amounts from the token accounts of the user to the vaults
        Transfer {
            from: self.accounts.vault_x,
            to: self.accounts.user_x_ata,
            authority: self.accounts.config,
            amount: x,
        }
        .invoke_signed(&signer)?;

        Transfer {
            from: self.accounts.vault_y,
            to: self.accounts.user_y_ata,
            authority: self.accounts.config,
            amount: y,
        }
        .invoke_signed(&signer)?;

        Burn {
            account: self.accounts.user_lp_ata,
            mint: self.accounts.mint_lp,
            authority: self.accounts.user,
            amount: self.instruction_data.amount,
        }
        .invoke()?;

        Ok(())
    }
}
