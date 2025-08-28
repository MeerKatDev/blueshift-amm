use core::mem::size_of;
use pinocchio::account_info::{Ref, RefMut};
use pinocchio::{account_info::AccountInfo, program_error::ProgramError, pubkey::Pubkey};

#[repr(C)]
pub struct Config {
    /// Tracks the status through AmmState
    state: u8,
    /// A unique value allowing for different configurations to exist
    seed: [u8; 8],
    /// Administrator of this Config'd pool, immutable when [0u8; 32]
    authority: Pubkey,
    /// SPL token mint address for token X
    mint_x: Pubkey,
    /// SPL token mint address for token Y
    mint_y: Pubkey,
    /// Swap collected and distributed to liquidity providers
    fee: [u8; 2],
    /// Bump seed for PDA derivation
    config_bump: [u8; 1],
}

#[repr(u8)]
#[derive(PartialEq, Eq)]
pub enum AmmState {
    Uninitialized = 0u8,
    Initialized = 1u8,
    Disabled = 2u8,
    WithdrawOnly = 3u8,
}

impl AmmState {
    pub fn is_initialized(self) -> bool {
        matches!(self, AmmState::Initialized)
    }
}

impl Config {
    pub const LEN: usize = size_of::<u8>()
        + size_of::<u64>()
        + size_of::<Pubkey>() * 3
        + size_of::<u16>()
        + size_of::<u8>();

    #[inline(always)]
    pub fn load(account_info: &AccountInfo) -> Result<Ref<Self>, ProgramError> {
        if account_info.data_len() != Self::LEN {
            return Err(ProgramError::InvalidAccountData);
        }
        if account_info.owner().ne(&crate::ID) {
            return Err(ProgramError::InvalidAccountOwner);
        }
        Ok(Ref::map(account_info.try_borrow_data()?, |data| unsafe {
            Self::from_bytes_unchecked(data)
        }))
    }

    /// # Safety
    ///
    /// blah blah blah
    #[inline(always)]
    pub unsafe fn load_unchecked(account_info: &AccountInfo) -> Result<&Self, ProgramError> {
        if account_info.data_len() != Self::LEN {
            return Err(ProgramError::InvalidAccountData);
        }
        if account_info.owner() != &crate::ID {
            return Err(ProgramError::InvalidAccountOwner);
        }
        Ok(Self::from_bytes_unchecked(
            account_info.borrow_data_unchecked(),
        ))
    }

    /// Return a `Config` from the given bytes.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `bytes` contains a valid representation of `Config`, and
    /// it is properly aligned to be interpreted as an instance of `Config`.
    /// At the moment `Config` has an alignment of 1 byte (meaning no padding used).
    /// This method does not perform a length validation.
    #[inline(always)]
    pub unsafe fn from_bytes_unchecked(bytes: &[u8]) -> &Self {
        &*(bytes.as_ptr() as *const Config)
    }

    /// Return a mutable `Config` reference from the given bytes.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `bytes` contains a valid representation of `Config`.
    #[inline(always)]
    pub unsafe fn from_bytes_unchecked_mut(bytes: &mut [u8]) -> &mut Self {
        &mut *(bytes.as_mut_ptr() as *mut Config)
    }

    // Getter methods for safe field access
    #[inline(always)]
    pub fn state(&self) -> u8 {
        self.state
    }

    #[inline(always)]
    pub fn seed(&self) -> u64 {
        u64::from_le_bytes(self.seed)
    }

    #[inline(always)]
    pub fn authority(&self) -> &Pubkey {
        &self.authority
    }

    #[inline(always)]
    pub fn mint_x(&self) -> &Pubkey {
        &self.mint_x
    }

    #[inline(always)]
    pub fn mint_y(&self) -> &Pubkey {
        &self.mint_y
    }

    #[inline(always)]
    pub fn fee(&self) -> u16 {
        u16::from_le_bytes(self.fee)
    }

    #[inline(always)]
    pub fn config_bump(&self) -> [u8; 1] {
        self.config_bump
    }

    #[inline(always)]
    pub fn load_mut(account_info: &AccountInfo) -> Result<RefMut<Self>, ProgramError> {
        if account_info.data_len() != Self::LEN {
            return Err(ProgramError::InvalidAccountData);
        }
        if account_info.owner().ne(&crate::ID) {
            return Err(ProgramError::InvalidAccountOwner);
        }
        Ok(RefMut::map(
            account_info.try_borrow_mut_data()?,
            |data| unsafe { Self::from_bytes_unchecked_mut(data) },
        ))
    }

    #[inline(always)]
    pub fn set_state(&mut self, state: u8) -> Result<(), ProgramError> {
        if state.ge(&(AmmState::WithdrawOnly as u8)) {
            return Err(ProgramError::InvalidAccountData);
        }
        self.state = state;
        Ok(())
    }

    #[inline(always)]
    pub fn set_fee(&mut self, fee: u16) -> Result<(), ProgramError> {
        if fee.ge(&10_000) {
            return Err(ProgramError::InvalidAccountData);
        }
        self.fee = fee.to_le_bytes();
        Ok(())
    }

    #[inline(always)]
    pub fn set_authority(&mut self, authority: Pubkey) -> Result<(), ProgramError> {
        if authority == Pubkey::default() {
            return Err(ProgramError::InvalidArgument);
        }
        self.authority = authority;
        Ok(())
    }

    #[inline(always)]
    pub fn set_mint_x(&mut self, mint_x: Pubkey) -> Result<(), ProgramError> {
        if mint_x == self.mint_y {
            return Err(ProgramError::InvalidArgument);
        }
        self.mint_x = mint_x;
        Ok(())
    }

    #[inline(always)]
    pub fn set_mint_y(&mut self, mint_y: Pubkey) -> Result<(), ProgramError> {
        if mint_y == self.mint_x {
            return Err(ProgramError::InvalidArgument);
        }
        self.mint_y = mint_y;
        Ok(())
    }

    #[inline(always)]
    pub fn set_seed(&mut self, seed: u64) -> Result<(), ProgramError> {
        // Seed should not be zero (avoid trivial PDA collisions)
        if seed == 0 {
            return Err(ProgramError::InvalidArgument);
        }
        // Optional: prevent re-setting once initialized
        if u64::from_le_bytes(self.seed) != 0 {
            return Err(ProgramError::AccountAlreadyInitialized);
        }

        self.seed = seed.to_le_bytes();

        Ok(())
    }

    #[inline(always)]
    pub fn set_config_bump(&mut self, bump: [u8; 1]) -> Result<(), ProgramError> {
        // Optional: enforce immutability after first set
        if self.config_bump != [0] {
            return Err(ProgramError::AccountAlreadyInitialized);
        }
        self.config_bump = bump;
        Ok(())
    }

    /// Atomic update - all fields are updated at once,
    /// so there's no risk of  inconsistencies
    #[inline(always)]
    pub fn set_inner(
        &mut self,
        seed: u64,
        authority: Pubkey,
        mint_x: Pubkey,
        mint_y: Pubkey,
        fee: u16,
        config_bump: [u8; 1],
    ) -> Result<(), ProgramError> {
        self.set_state(AmmState::Initialized as u8)?;
        self.set_seed(seed)?;
        self.set_authority(authority)?;
        self.set_mint_x(mint_x)?;
        self.set_mint_y(mint_y)?;
        self.set_fee(fee)?;
        self.set_config_bump(config_bump)?;
        Ok(())
    }

    /// efficient way to check whether authority is set or
    /// it's made of zeroes
    #[inline(always)]
    pub fn has_authority(&self) -> Option<Pubkey> {
        let bytes = self.authority();
        let chunks: &[u64; 4] = unsafe { &*(bytes.as_ptr() as *const [u64; 4]) };
        if chunks.iter().any(|&x| x != 0) {
            Some(self.authority)
        } else {
            None
        }
    }
}
