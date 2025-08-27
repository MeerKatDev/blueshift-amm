use crate::Config;
use pinocchio::account_info::AccountInfo;
use pinocchio::instruction::Seed;
use pinocchio::instruction::Signer;
use pinocchio::program_error::ProgramError;
use pinocchio::sysvars::rent::Rent;
use pinocchio::sysvars::Sysvar;
use pinocchio::ProgramResult;
use pinocchio_system::instructions::CreateAccount;
use pinocchio_token::id as token_program_id;
use pinocchio_token::instructions::InitializeMint2;
use std::mem::MaybeUninit;
// use pinocchio::msg;
use pinocchio_log::log;
use pinocchio_token::state::Mint;

// This instruction initializes the pool.
// In order:
// - it initializes the Config state
// - creates the Mint account `mint_lp` for the pool tokens
// - assigns the mint authority

pub struct InitializeAccounts<'a> {
    /// Creator, not necessarily the authority over it
    pub initializer: &'a AccountInfo,
    /// Mint representing pool liquidity tokens
    pub mint_lp: &'a AccountInfo,
    pub config: &'a AccountInfo,
}

impl<'a> TryFrom<&'a [AccountInfo]> for InitializeAccounts<'a> {
    type Error = ProgramError;

    fn try_from(accounts: &'a [AccountInfo]) -> Result<Self, Self::Error> {
        let [initializer, mint_lp, config, _, _] = accounts else {
            log!("{}", format!("accounts: {:?}", accounts.len()).as_str());
            return Err(ProgramError::NotEnoughAccountKeys);
        };

        // Basic Accounts Checks
        // SignerAccount::check(initializer)?;
        // MintInterface::check(mint_lp)?;

        // check initializer is signer
        // but it will be checked downstream maybe?

        Ok(Self {
            initializer,
            mint_lp,
            config,
        })
    }
}

#[repr(C, packed)]
pub struct InitializeInstructionData {
    pub seed: u64,
    pub fee: u16,
    pub mint_x: [u8; 32],
    pub mint_y: [u8; 32],
    pub config_bump: [u8; 1],
    pub lp_bump: [u8; 1],
    /// omittable for immutable pool
    pub authority: [u8; 32],
}

impl TryFrom<&[u8]> for InitializeInstructionData {
    type Error = ProgramError;

    fn try_from(data: &[u8]) -> Result<Self, Self::Error> {
        const INITIALIZE_DATA_LEN_WITH_AUTHORITY: usize = size_of::<InitializeInstructionData>();
        const INITIALIZE_DATA_LEN: usize =
            INITIALIZE_DATA_LEN_WITH_AUTHORITY - size_of::<[u8; 32]>();

        match data.len() {
            INITIALIZE_DATA_LEN_WITH_AUTHORITY => {
                Ok(unsafe { (data.as_ptr() as *const Self).read_unaligned() })
            }
            INITIALIZE_DATA_LEN => {
                // If the authority is not present, we need to build the buffer and add it at the end before transmuting to the struct
                let mut raw: MaybeUninit<[u8; INITIALIZE_DATA_LEN]> = MaybeUninit::uninit();
                let raw_ptr = raw.as_mut_ptr() as *mut u8;
                unsafe {
                    // Copy the provided data
                    core::ptr::copy_nonoverlapping(data.as_ptr(), raw_ptr, INITIALIZE_DATA_LEN);
                    // Add the authority to the end of the buffer
                    core::ptr::write_bytes(raw_ptr.add(INITIALIZE_DATA_LEN), 0, 32);
                    // Now transmute to the struct
                    Ok((raw.as_ptr() as *const Self).read_unaligned())
                }
            }
            _ => Err(ProgramError::InvalidInstructionData),
        }
    }
}

pub struct Initialize<'a> {
    pub accounts: InitializeAccounts<'a>,
    pub instruction_data: InitializeInstructionData,
}

impl<'a> TryFrom<(&'a [u8], &'a [AccountInfo])> for Initialize<'a> {
    type Error = ProgramError;

    fn try_from((data, accounts): (&'a [u8], &'a [AccountInfo])) -> Result<Self, Self::Error> {
        let accounts = InitializeAccounts::try_from(accounts)?;
        let instruction_data: InitializeInstructionData =
            InitializeInstructionData::try_from(data)?;

        Ok(Self {
            accounts,
            instruction_data,
        })
    }
}

impl<'a> Initialize<'a> {
    pub const DISCRIMINATOR: &'a u8 = &0;

    pub fn process(&mut self) -> ProgramResult {
        let seed_binding = self.instruction_data.seed.to_le_bytes();
        let config_seeds = &[
            Seed::from(b"config"),
            Seed::from(&seed_binding),
            Seed::from(&self.instruction_data.mint_x),
            Seed::from(&self.instruction_data.mint_y),
            Seed::from(&self.instruction_data.config_bump),
        ];

        // Get required lamports for rent
        let lamports = Rent::get()?.minimum_balance(Config::LEN);

        // Create signer with seeds slice
        let signer = [Signer::from(config_seeds)];

        // Create the account
        CreateAccount {
            from: self.accounts.initializer,
            to: self.accounts.config,
            lamports,
            space: Config::LEN as u64,
            owner: &crate::ID,
        }
        .invoke_signed(&signer)?;

        let mint_lp_seeds = &[
            Seed::from(b"mint_lp"),
            Seed::from(self.accounts.config.key()),
            Seed::from(&self.instruction_data.lp_bump),
        ];

        // Create signer with seeds slice
        let mint_signer = [Signer::from(mint_lp_seeds)];

        // Create the LP mint account
        let lamports = Rent::get()?.minimum_balance(Mint::LEN);

        CreateAccount {
            from: self.accounts.initializer,
            to: self.accounts.mint_lp,
            lamports,
            space: Mint::LEN as u64,
            owner: &token_program_id(),
        }
        .invoke_signed(&mint_signer)?;

        InitializeMint2 {
            mint: self.accounts.mint_lp,
            decimals: 6,
            mint_authority: self.accounts.config.key(),
            freeze_authority: None,
        }
        .invoke_signed(&mint_signer)?;

        Ok(())
    }
}
