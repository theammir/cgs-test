use anchor_lang::prelude::*;

declare_id!("3N6Aj5juQwPebTDtxxc38GPLo2kGeDKmzVWJqN9y2Hiv");

#[program]
pub mod cgs_test {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        msg!("Greetings from: {:?}", ctx.program_id);
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize {}
