use anchor_lang::prelude::*;
use anchor_lang::solana_program::rent::Rent;
use anchor_lang::solana_program::{
    program::{invoke, invoke_signed},
    system_instruction,
};
use anchor_lang::solana_program::system_program;

#[cfg(not(feature = "no-entrypoint"))]
use solana_security_txt::security_txt;

#[cfg(not(feature = "no-entrypoint"))]
security_txt! {
    name: "Treasury Escrow",
    project_url: "https://jigsaw.chat",
    contacts: "email:security@jigsaw.chat",
    policy: "https://jigsaw.chat/security-policy",
    source_code: "https://github.com/tycoonscripts/jigsaw"
}

// Temporary placeholder Program ID for building. Replace with your deployed program ID.
declare_id!("6GabEnTZtPMyUDkrbzEMDktDupZ3gxVWb6oEHBsoRZ61");

#[program]
pub mod treasury_escrow {
    use super::*;

    pub fn initialize(
        ctx: Context<Initialize>,
        base_fee: u64,
        fee_cap: u64,
        marketing_bps: u16,
    ) -> Result<()> {
        let escrow = &mut ctx.accounts.escrow;
    
        // -------------------------------------------------
        // 1. Create the vault PDA account manually
        // -------------------------------------------------
    
        // how much rent-exempt lamports for an account with 0 data bytes
        let rent_lamports = Rent::get()?.minimum_balance(0);
    
        // bump for vault PDA
        let vault_bump = ctx.bumps.escrow_vault;
    
        // seeds we will sign with for the new account
        let escrow_seed: &[u8] = b"escrow";
        let vault_seed: &[u8] = b"vault";
        let bump_seed: &[u8] = &[vault_bump];
        let signer_seeds: &[&[u8]] = &[escrow_seed, vault_seed, bump_seed];
    
        // build the `create_account` ix:
        // - `authority` funds it
        // - new account is `escrow_vault`
        // - owner is this program (crate::ID)
        invoke_signed(
            &system_instruction::create_account(
                &ctx.accounts.authority.key(),               // from
                &ctx.accounts.escrow_vault.key(),            // new account pubkey (the PDA)
                rent_lamports,                               // lamports
                0,                                           // space in bytes
                &system_program::ID,                                  // owner = this program
            ),
            &[
                ctx.accounts.authority.to_account_info(),
                ctx.accounts.escrow_vault.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
            &[signer_seeds],
        )?;
    
        // -------------------------------------------------
        // 2. Initialize escrow state
        // -------------------------------------------------
        escrow.authority = ctx.accounts.authority.key();
        escrow.base_fee = base_fee;
        escrow.fee_cap = fee_cap;
        escrow.current_fee = base_fee;
        escrow.marketing_wallet = ctx.accounts.marketing_wallet.key();
        escrow.marketing_bps = marketing_bps;
        escrow.messages_count = 0;
        escrow.last_sender = Pubkey::default();
        escrow.timer_active = false;
        escrow.deadline = 0;
        escrow.ended = false;
        escrow.bump = ctx.bumps.escrow;
    
        Ok(())
    }

    pub fn submit_message(ctx: Context<SubmitMessage>, msg_hash: [u8; 32]) -> Result<()> {
        let escrow = &mut ctx.accounts.escrow;
        let clock = Clock::get()?;
        let fee_paid = escrow.current_fee;
    
        // 1. game still live?
        require!(!escrow.ended, ErrorCode::GameEnded);
        require_keys_eq!(
            ctx.accounts.marketing_wallet.key(),
            escrow.marketing_wallet,
            ErrorCode::Unauthorized
        );
    
        // 2. timer not expired if active
        if escrow.timer_active {
            require!(
                clock.unix_timestamp <= escrow.deadline,
                ErrorCode::TimerExpired
            );
        }
    
        // 3. sanity: payer can afford the fee
        let payer_lamports = ctx.accounts.payer.lamports();
        require!(payer_lamports >= escrow.current_fee, ErrorCode::InsufficientFee);
    
        // -------------------------------------------------
        // 4. compute splits
        // -------------------------------------------------
        // marketing_fee = current_fee * bps / 10000
        let marketing_fee: u64 = (escrow.current_fee as u128)
            .checked_mul(escrow.marketing_bps as u128)
            .unwrap()
            .checked_div(10_000)
            .unwrap() as u64;
    
        // prize portion is whatever's left after marketing skim
        let prize_fee: u64 = escrow
            .current_fee
            .checked_sub(marketing_fee)
            .unwrap();
    
        // -------------------------------------------------
        // 5. payer -> escrow_vault (the prize pool)
        // -------------------------------------------------
        if prize_fee > 0 {
            invoke(
                &system_instruction::transfer(
                    &ctx.accounts.payer.key(),
                    &ctx.accounts.escrow_vault.key(),
                    prize_fee,
                ),
                &[
                    ctx.accounts.payer.to_account_info(),
                    ctx.accounts.escrow_vault.to_account_info(),
                    ctx.accounts.system_program.to_account_info(),
                ],
            )?;
        }
    
        // -------------------------------------------------
        // 6. payer -> marketing_wallet (the rake)
        // -------------------------------------------------
        if marketing_fee > 0 && escrow.marketing_wallet != Pubkey::default() {
            invoke(
                &system_instruction::transfer(
                    &ctx.accounts.payer.key(),
                    &ctx.accounts.marketing_wallet.key(),
                    marketing_fee,
                ),
                &[
                    ctx.accounts.payer.to_account_info(),
                    ctx.accounts.marketing_wallet.to_account_info(), // <-- add this
                    ctx.accounts.system_program.to_account_info(),
                ],
            )?;
            emit!(MarketingFeeSent { wallet: ctx.accounts.marketing_wallet.key(), amount: marketing_fee });
        }        
    
        // -------------------------------------------------
        // 7. update on-chain state
        // -------------------------------------------------
        escrow.messages_count = escrow.messages_count.checked_add(1).unwrap();
        escrow.last_sender = ctx.accounts.payer.key();
    
        // timer rules
        const START_AFTER: u64 = 10;
        const EXTEND_SECONDS: i64 = 3600;
    
        let mut timer_started = false;
        let mut timer_extended = false;
    
        if !escrow.timer_active && escrow.messages_count >= START_AFTER {
            escrow.timer_active = true;
            escrow.deadline = clock.unix_timestamp.checked_add(EXTEND_SECONDS).unwrap();
            timer_started = true;
        } else if escrow.timer_active && clock.unix_timestamp <= escrow.deadline {
            escrow.deadline = clock.unix_timestamp.checked_add(EXTEND_SECONDS).unwrap();
            timer_extended = true;
        }
    
        // -------------------------------------------------
        // 8. bump the dynamic fee, capped
        // -------------------------------------------------
        let next_fee = (escrow.current_fee as u128)
            .checked_mul(10078)
            .unwrap()
            .checked_div(10000)
            .unwrap() as u64;
    
        escrow.current_fee = if next_fee > escrow.fee_cap {
            escrow.fee_cap
        } else {
            next_fee
        };
    
        // -------------------------------------------------
        // 9. emit events
        // -------------------------------------------------
        emit!(MessageSubmitted {
            sender: ctx.accounts.payer.key(),
            msg_hash,
            fee_paid,
            new_fee: escrow.current_fee,
            timestamp: clock.unix_timestamp,
        });
    
        if timer_started {
            emit!(TimerStarted {
                deadline: escrow.deadline,
            });
        } else if timer_extended {
            emit!(TimerExtended {
                new_deadline: escrow.deadline,
            });
        }
    
        Ok(())
    }    

    pub fn claim_prize(ctx: Context<ClaimPrize>) -> Result<()> {
        let escrow = &mut ctx.accounts.escrow;
        let clock = Clock::get()?;
    
        require!(escrow.timer_active, ErrorCode::GameNotEnded);
        require!(clock.unix_timestamp >= escrow.deadline, ErrorCode::GameNotEnded);
        require!(escrow.last_sender != Pubkey::default(), ErrorCode::NoWinner);
        require!(!escrow.ended, ErrorCode::AlreadyClaimed);
        require!(ctx.accounts.winner.key() == escrow.last_sender, ErrorCode::NotTheWinner);
    
        escrow.ended = true;
    
        // How much is in the vault right now?
        let balance = ctx.accounts.escrow_vault.lamports();
    
        // Transfer all lamports from vault PDA → winner using invoke_signed
        // (SystemProgram transfer signed by vault PDA seeds)
        let bump = ctx.bumps.escrow_vault;
    
        let escrow_seed: &[u8] = b"escrow";
        let vault_seed: &[u8] = b"vault";
        let bump_seed: &[u8] = &[bump];
    
        let signer_seeds: &[&[u8]] = &[escrow_seed, vault_seed, bump_seed];
    
        invoke_signed(
            &system_instruction::transfer(
                &ctx.accounts.escrow_vault.key(),
                &ctx.accounts.winner.key(),
                balance,
            ),
            &[
                ctx.accounts.escrow_vault.to_account_info(),
                ctx.accounts.winner.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
            &[signer_seeds],
        )?;
    
        emit!(PrizeClaimed {
            winner: ctx.accounts.winner.key(),
            amount: balance,
        });
    
        Ok(())
    }
    

    pub fn jigsaw_approve_payout(ctx: Context<JigsawApprovePayout>) -> Result<()> {
        let escrow = &mut ctx.accounts.escrow;
    
        // --- validity checks ---
        require!(!escrow.ended, ErrorCode::AlreadyClaimed);
        require!(
            ctx.accounts.winner.key() == escrow.last_sender,
            ErrorCode::NotTheWinner
        );
        require!(escrow.last_sender != Pubkey::default(), ErrorCode::NoWinner);
    
        // Mark game as ended so it can't be claimed twice
        escrow.ended = true;
    
        // Read how many lamports are currently in the vault
        let balance = ctx.accounts.escrow_vault.lamports();
    
        // Build signer seeds for the vault PDA
        // vault PDA is seeds = [b"escrow", b"vault"], bump = ctx.bumps.escrow_vault
        let bump = ctx.bumps.escrow_vault;
    
        let escrow_seed: &[u8] = b"escrow";
        let vault_seed: &[u8] = b"vault";
        let bump_seed: &[u8] = &[bump];
    
        let signer_seeds: &[&[u8]] = &[escrow_seed, vault_seed, bump_seed];
    
        // Transfer the entire vault balance to the winner using CPI
        // The vault PDA signs this transfer via invoke_signed
        invoke_signed(
            &system_instruction::transfer(
                &ctx.accounts.escrow_vault.key(),
                &ctx.accounts.winner.key(),
                balance,
            ),
            &[
                ctx.accounts.escrow_vault.to_account_info(),
                ctx.accounts.winner.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
            &[signer_seeds],
        )?;
    
        // Emit event for indexing / frontend
        emit!(PrizeClaimed {
            winner: ctx.accounts.winner.key(),
            amount: balance,
        });
    
        Ok(())
    }    

    pub fn set_fee_params(ctx: Context<SetFeeParams>, base_fee: u64, fee_cap: u64) -> Result<()> {
        let escrow = &mut ctx.accounts.escrow;
        
        require!(base_fee > 0 && base_fee <= fee_cap, ErrorCode::BadParams);
        
        escrow.base_fee = base_fee;
        escrow.fee_cap = fee_cap;
        
        if escrow.current_fee < base_fee {
            escrow.current_fee = base_fee;
        }
        if escrow.current_fee > fee_cap {
            escrow.current_fee = fee_cap;
        }

        Ok(())
    }

    pub fn set_marketing_params(
        ctx: Context<SetMarketingParams>,
        wallet: Pubkey,
        bps: u16,
    ) -> Result<()> {
        let escrow = &mut ctx.accounts.escrow;
        
        require!(bps <= 2500, ErrorCode::BpsTooHigh);
        
        escrow.marketing_wallet = wallet;
        escrow.marketing_bps = bps;

        emit!(MarketingParamsUpdated {
            wallet,
            bps,
        });

        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        init,
        payer = authority,
        space = 8 + Escrow::LEN,
        seeds = [b"escrow"],
        bump
    )]
    pub escrow: Account<'info, Escrow>,

    /// CHECK:
    /// This PDA will be created in this instruction via `create_account`
    /// and owned by this program. We don't read or trust any preexisting data.
    #[account(
        mut,
        seeds = [b"escrow", b"vault"],
        bump
    )]
    pub escrow_vault: SystemAccount<'info>,

    /// CHECK: arbitrary marketing wallet set by the authority; not controlled by program
    pub marketing_wallet: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SubmitMessage<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(
        mut,
        seeds = [b"escrow"],
        bump = escrow.bump
    )]
    pub escrow: Account<'info, Escrow>,

    /// CHECK:
    /// This is the vault PDA (seeds ["escrow","vault"]) created in `initialize`.
    /// It is owned by our program (not the system program) and just holds lamports.
    /// We only ever move lamports via CPI using invoke_signed(), so we trust seeds+bump
    /// instead of Anchor's owner check.
    #[account(
        mut,
        seeds = [b"escrow", b"vault"],
        bump
    )]
    pub escrow_vault: SystemAccount<'info>,

    /// CHECK:
    /// This is the marketing wallet set by the authority; not controlled by program
    /// Must be the wallet stored in escrow
    #[account(
        mut,
        address = escrow.marketing_wallet @ ErrorCode::Unauthorized
    )]
    pub marketing_wallet: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ClaimPrize<'info> {
    #[account(mut)]
    pub winner: Signer<'info>,

    #[account(
        mut,
        seeds = [b"escrow"],
        bump = escrow.bump
    )]
    pub escrow: Account<'info, Escrow>,

    /// CHECK:
    /// Program-owned vault PDA that holds the pooled lamports.
    /// We'll sign for it with [b"escrow", b"vault", bump] and transfer out all lamports.
    #[account(
        mut,
        seeds = [b"escrow", b"vault"],
        bump
    )]
    pub escrow_vault: SystemAccount<'info>,

    pub system_program: Program<'info, System>,
}


#[derive(Accounts)]
pub struct JigsawApprovePayout<'info> {
    /// CHECK:
    /// authority == jigsaw_approver in the `escrow` constraint, so this is safe.
    pub jigsaw_approver: Signer<'info>,

    #[account(
        mut,
        seeds = [b"escrow"],
        bump = escrow.bump,
        constraint = escrow.authority == jigsaw_approver.key() @ ErrorCode::Unauthorized
    )]
    pub escrow: Account<'info, Escrow>,

    /// CHECK:
    /// `winner` is just the payout destination. We never read or mutate its data,
    /// we only send lamports to it via a system transfer. So it does not need to
    /// satisfy any owner/type invariants.
    #[account(mut)]
    pub winner: UncheckedAccount<'info>,

    /// CHECK:
    /// `escrow_vault` is the program-owned PDA `[b"escrow", b"vault"]` that holds
    /// the prize pool lamports. We sign for it with `invoke_signed` using those seeds.
    #[account(
        mut,
        seeds = [b"escrow", b"vault"],
        bump
    )]
    pub escrow_vault: SystemAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SetFeeParams<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    
    #[account(
        mut,
        seeds = [b"escrow"],
        bump = escrow.bump,
        constraint = escrow.authority == authority.key() @ ErrorCode::Unauthorized
    )]
    pub escrow: Account<'info, Escrow>,
}

#[derive(Accounts)]
pub struct SetMarketingParams<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    
    #[account(
        mut,
        seeds = [b"escrow"],
        bump = escrow.bump,
        constraint = escrow.authority == authority.key() @ ErrorCode::Unauthorized
    )]
    pub escrow: Account<'info, Escrow>,
    
    /// CHECK: New marketing wallet
    pub marketing_wallet: UncheckedAccount<'info>,
}

#[account]
pub struct Escrow {
    pub authority: Pubkey,
    pub base_fee: u64,
    pub fee_cap: u64,
    pub current_fee: u64,
    pub marketing_wallet: Pubkey,
    pub marketing_bps: u16,
    pub messages_count: u64,
    pub last_sender: Pubkey,
    pub timer_active: bool,
    pub deadline: i64,
    pub ended: bool,
    pub bump: u8,
}

impl Escrow {
    pub const LEN: usize = 32 + 8 + 8 + 8 + 32 + 2 + 8 + 32 + 1 + 8 + 1 + 1;
}

#[event]
pub struct MessageSubmitted {
    pub sender: Pubkey,
    pub msg_hash: [u8; 32],
    pub fee_paid: u64,
    pub new_fee: u64,
    pub timestamp: i64,
}

#[event]
pub struct TimerStarted {
    pub deadline: i64,
}

#[event]
pub struct TimerExtended {
    pub new_deadline: i64,
}

#[event]
pub struct MarketingFeeSent {
    pub wallet: Pubkey,
    pub amount: u64,
}

#[event]
pub struct MarketingParamsUpdated {
    pub wallet: Pubkey,
    pub bps: u16,
}

#[event]
pub struct PrizeClaimed {
    pub winner: Pubkey,
    pub amount: u64,
}

#[error_code]
pub enum ErrorCode {
    #[msg("Game ended")]
    GameEnded,
    #[msg("Timer expired")]
    TimerExpired,
    #[msg("Insufficient fee")]
    InsufficientFee,
    #[msg("Game not ended")]
    GameNotEnded,
    #[msg("Already claimed")]
    AlreadyClaimed,
    #[msg("Not the winner")]
    NotTheWinner,
    #[msg("No winner")]
    NoWinner,
    #[msg("Bad params")]
    BadParams,
    #[msg("Bps too high")]
    BpsTooHigh,
    #[msg("Unauthorized")]
    Unauthorized,
}

