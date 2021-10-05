import { TOKEN_PROGRAM_ID } from "@solana/spl-token";
import {
  PublicKey,
  SYSVAR_CLOCK_PUBKEY,
  TransactionInstruction,
} from "@solana/web3.js";
import { struct, u8 } from "buffer-layout";
import { LENDING_PROGRAM_ID } from "../constants";
import { u64 } from "../util";
import { LendingInstruction } from "./instruction";

interface Data {
  instruction: number;
  liquidityAmount: bigint;
}

const DataLayout = struct<Data>([u8("instruction"), u64("liquidityAmount")]);

export const depositReserveLiquidityAndObligationCollateralInstruction = (
  liquidityAmount: number | bigint,
  sourceLiquidity: PublicKey,
  destinationCollateral: PublicKey,
  reserve: PublicKey,
  reserveLiquiditySupply: PublicKey,
  reserveCollateralMint: PublicKey,
  lendingMarket: PublicKey,
  lendingMarketAuthority: PublicKey,
  depositReserve: PublicKey,
  obligation: PublicKey,
  obligationOwner: PublicKey,
  oracle: PublicKey,
  switchboardFeed: PublicKey,
  transferAuthority: PublicKey
): TransactionInstruction => {
  const data = Buffer.alloc(DataLayout.span);
  DataLayout.encode(
    {
      instruction: LendingInstruction.DepositReserveLiquidityAndObligationCollateral,
      liquidityAmount: BigInt(liquidityAmount),
    },
    data
  );

  const keys = [
    ///   0. `[writable]` Source liquidity token account.
    { pubkey: sourceLiquidity, isSigner: false, isWritable: true },
    ///   1. `[writable]` Destination collateral token account.
    { pubkey: destinationCollateral, isSigner: false, isWritable: true },
    ///   2. `[writable]` Reserve account.
    { pubkey: reserve, isSigner: false, isWritable: true },
    ///   3. `[writable]` Reserve liquidity supply SPL Token account.
    { pubkey: reserveLiquiditySupply, isSigner: false, isWritable: true },
    ///   4. `[writable]` Reserve collateral SPL Token mint.
    { pubkey: reserveCollateralMint, isSigner: false, isWritable: true },
    ///   5. `[]` Lending market account.
    { pubkey: lendingMarket, isSigner: false, isWritable: false },
    ///   6. `[]` Derived lending market authority.
    { pubkey: lendingMarketAuthority, isSigner: false, isWritable: false },
    ///   7. `[writable]` Destination deposit reserve collateral supply SPL Token account.
    { pubkey: depositReserve, isSigner: false, isWritable: true },
    ///   8. `[writable]` Obligation account.
    { pubkey: obligation, isSigner: false, isWritable: true },
    ///   9. `[signer]` Obligation owner.
    { pubkey: obligationOwner, isSigner: true, isWritable: false },
    ///   10 `[]` Pyth price oracle account.
    { pubkey: oracle, isSigner: false, isWritable: false },
    ///   11 `[]` Switchboard price feed oracle account.
    { pubkey: switchboardFeed, isSigner: false, isWritable: false },
    ///   12 `[signer]` User transfer authority ($authority).
    { pubkey: transferAuthority, isSigner: true, isWritable: false },
    ///   13 `[]` Clock sysvar.
    { pubkey: SYSVAR_CLOCK_PUBKEY, isSigner: false, isWritable: false },
    ///   14 `[]` Token program id.
    { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
  ];

  return new TransactionInstruction({
    keys,
    programId: LENDING_PROGRAM_ID,
    data,
  });
};
