# Treasury Escrow Program

**Program ID:** `6GabEnTZtPMyUDkrbzEMDktDupZ3gxVWb6oEHBsoRZ61`  
**Project:** [JigSaw AI](https://jigsaw.chat)  
**Security Contact:** email:security@jigsaw.chat  
**Security Policy:** https://jigsaw.chat/security-policy

## Overview

This Solana program implements a transparent, on-chain escrow system for JigSaw AI's message submission and prize distribution mechanism. The program operates as a trustless escrow that pools fees from participants and distributes prizes based on deterministic, on-chain rules.

## Program Intentions

The Treasury Escrow program is designed to:

1. **Provide transparent prize distribution** - All prize pool funds are held in a program-controlled PDA (Program Derived Address) that cannot be accessed by any single party
2. **Enable fair competition** - Participants compete in a "last sender wins" game with clear, verifiable rules
3. **Maintain operational sustainability** - A configurable marketing fee supports the platform's operations while keeping the majority of fees in the prize pool

## How It Works

### Core Mechanism

1. **Initialization**: The escrow is initialized with:
   - `base_fee`: Starting fee amount (in lamports)
   - `fee_cap`: Maximum fee that can be charged
   - `marketing_bps`: Basis points (0-2500, max 25%) for marketing fee split

2. **Message Submission** (`submit_message`):
   - Users pay the current fee to submit a message (represented as a 32-byte hash)
   - Fees are split:
     - Marketing portion → `marketing_wallet` (configurable by authority)
     - Prize portion → `escrow_vault` PDA (the prize pool)
   - After 10 messages, a 1-hour timer activates
   - Each subsequent message extends the timer by 1 hour
   - The fee increases by 0.78% per message (capped at `fee_cap`)
   - The last sender before timer expiration becomes the winner

3. **Prize Claiming**:
   - **Automatic claim** (`claim_prize`): When the timer expires, the last sender can claim the prize
   - **AI-approved claim** (`eve_approve_payout`): The authority (Eve AI/TEE wallet) can approve payouts, enabling additional verification or off-chain checks (e.g., Worldcoin Orb verification)

### Key State Variables

- `authority`: The program authority (Eve AI/TEE wallet)
- `base_fee` / `fee_cap`: Fee bounds
- `current_fee`: Dynamic fee that increases per submission
- `marketing_wallet` / `marketing_bps`: Marketing fee configuration
- `messages_count`: Total messages submitted
- `last_sender`: The current winner (last sender)
- `timer_active`: Whether the countdown timer is active
- `deadline`: Unix timestamp when timer expires
- `ended`: Whether the game has ended and prize claimed

## Security Considerations

### On-Chain Security

1. **Program-Controlled Vault**: The prize pool is held in a PDA (`[b"escrow", b"vault"]`) that only the program can control. No single party can withdraw funds without following the program's rules.

2. **Access Controls**:
   - Only the `authority` can initialize and update fee/marketing parameters
   - Only the `last_sender` can claim via `claim_prize` after timer expiration
   - Only the `authority` can approve payouts via `eve_approve_payout` (but must still respect the `last_sender` rule)

3. **Reentrancy Protection**: The program marks `ended = true` before transferring funds, preventing double-claiming.

4. **Fee Validation**: All fee calculations use checked arithmetic to prevent overflow/underflow.

5. **Marketing Fee Cap**: Marketing fees are capped at 25% (2500 bps) to protect participants.

### Security.txt

This program includes a `security.txt` record (via `solana-security-txt`) that can be queried on-chain for security contact information.

### Audit Recommendations

For security auditors reviewing this program, pay special attention to:

1. **PDA Derivation**: Verify that all PDAs are correctly derived and cannot be controlled by external parties
2. **Fee Calculation**: Check that the 0.78% fee increase (`10078/10000`) and marketing split calculations are correct
3. **Timer Logic**: Ensure the timer start/extend logic correctly implements the game rules
4. **Access Control**: Verify that `eve_approve_payout` cannot be abused to bypass the `last_sender` requirement
5. **Account Validation**: Review all `UncheckedAccount` usages to ensure they're safe

## Program Instructions

| Instruction | Description | Authority Required |
|------------|-------------|-------------------|
| `initialize` | Initialize the escrow with fee and marketing parameters | Authority |
| `submit_message` | Submit a message and pay the current fee | Any user |
| `claim_prize` | Claim prize after timer expiration | Last sender |
| `eve_approve_payout` | Authority-approved payout (for additional verification) | Authority + Last sender |
| `set_fee_params` | Update base fee and fee cap | Authority |
| `set_marketing_params` | Update marketing wallet and fee percentage | Authority |

## Events

The program emits the following events for indexing and transparency:

- `MessageSubmitted`: Emitted on each message submission
- `TimerStarted`: Emitted when the timer first activates (after 10 messages)
- `TimerExtended`: Emitted when the timer is extended by a new message
- `MarketingFeeSent`: Emitted when marketing fees are transferred
- `MarketingParamsUpdated`: Emitted when marketing parameters change
- `PrizeClaimed`: Emitted when a prize is claimed

## Building and Testing

```bash
# Install dependencies
anchor build

# Run tests
anchor test
```

## Deployment

- **Mainnet/Devnet Program ID**: `GffTBQb9YjjFMPfLnqo8fqgDKKvam8keTRutRUvKux5p`
- **Network**: Solana Mainnet (configurable in `Anchor.toml`)

## Transparency and Verification

This program is open-source and designed for transparency:

1. **On-Chain Verification**: All program logic is executed on-chain and can be verified by inspecting transactions
2. **Deterministic Rules**: Prize distribution follows deterministic, verifiable rules
3. **Event Logging**: All significant actions emit events that can be indexed and monitored
4. **No Hidden Logic**: The entire program logic is visible in this repository

## Contact

- **Project**: https://eveai.chat
- **Security**: security@eveai.chat
- **Source Code**: https://github.com/tycoonscripts/eveai

## DEPLOYMENT

solana program deploy \
  -u "https://devnet.helius-rpc.com/?api-key=<API_KEY>" \
  --program-id target/deploy/treasury_escrow-keypair.json \
  --with-compute-unit-price 50000 \
  --max-sign-attempts 100 \
  --use-rpc \
  target/deploy/treasury_escrow.so
