import {
  PublicKey,
  SYSVAR_CLOCK_PUBKEY,
  TransactionInstruction,
} from '@solana/web3.js';
import BufferLayout from 'buffer-layout';
import { Config } from 'global';
import { LendingInstruction } from './instruction';

/// Refresh an obligation"s accrued interest and collateral and liquidity prices. Requires
/// refreshed reserves, as all obligation collateral deposit reserves in order, followed by all
/// liquidity borrow reserves in order.
/// Accounts expected by this instruction:
///   0. `[writable]` Obligation account.
///   1. `[]` Clock sysvar.
///   .. `[]` Collateral deposit reserve accounts - refreshed, all, in order.
///   .. `[]` Liquidity borrow reserve accounts - refreshed, all, in order.
export const refreshObligationInstruction = (
  config: Config,
  obligation: PublicKey,
  depositReserves: PublicKey[],
  borrowReserves: PublicKey[],
): TransactionInstruction => {
  const dataLayout = BufferLayout.struct([BufferLayout.u8('instruction')]);

  const data = Buffer.alloc(dataLayout.span);
  dataLayout.encode(
    { instruction: LendingInstruction.RefreshObligation },
    data,
  );

  const keys = [
    { pubkey: obligation, isSigner: false, isWritable: true },
    { pubkey: SYSVAR_CLOCK_PUBKEY, isSigner: false, isWritable: false },
  ];

  depositReserves.forEach((depositReserve) => {
    keys.push({ pubkey: depositReserve, isSigner: false, isWritable: false });
  });

  borrowReserves.forEach((borrowReserve) => {
    keys.push({ pubkey: borrowReserve, isSigner: false, isWritable: false });
  });

  return new TransactionInstruction({
    keys,
    programId: new PublicKey(config.programID),
    data,
  });
};
