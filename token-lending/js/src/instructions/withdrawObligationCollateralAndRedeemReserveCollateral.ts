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
  collateralAmount: bigint;
}

const DataLayout = struct<Data>([u8("instruction"), u64("collateralAmount")]);

export const withdrawObligationCollateralAndRedeemReserveCollateralInstruction = (
  collateralAmount: number | bigint,
  sourceCollateral: PublicKey,
  destinationCollateral: PublicKey,
  reserve: PublicKey,
  obligation: PublicKey,
  lendingMarket: PublicKey,
  lendingMarketAuthority: PublicKey,
  destinationLiquidity: PublicKey,
  reserveCollateralMint: PublicKey,
  reserveLiquiditySupply: PublicKey,
  obligationOwner: PublicKey,
  transferAuthority: PublicKey,
): TransactionInstruction => {
  const data = Buffer.alloc(DataLayout.span);
  DataLayout.encode(
    {
      instruction:
        LendingInstruction.WithdrawObligationCollateralAndRedeemReserveCollateral,
      collateralAmount: BigInt(collateralAmount),
    },
    data
  );

  const keys = [
    ///   0. `[writable]` Source withdraw reserve collateral supply SPL Token account.
    { pubkey: sourceCollateral, isSigner: false, isWritable: true },
    ///   1. `[writable]` Destination collateral token account.
    { pubkey: destinationCollateral, isSigner: false, isWritable: true },
    ///   2. `[]` Withdraw reserve account - refreshed.
    { pubkey: reserve, isSigner: false, isWritable: false },
    ///   3. `[writable]` Obligation account - refreshed.
    { pubkey: obligation, isSigner: false, isWritable: true },
    ///   4. `[]` Lending market account.
    { pubkey: lendingMarket, isSigner: false, isWritable: false },
    ///   5. `[]` Derived lending market authority.
    { pubkey: lendingMarketAuthority, isSigner: false, isWritable: false },
    ///   6. `[writable]` User liquidity token account.
    { pubkey: destinationLiquidity, isSigner: false, isWritable: true },
    ///   7. `[writable]` Reserve collateral SPL Token mint.
    { pubkey: reserveCollateralMint, isSigner: false, isWritable: true },
    ///   8. `[writable]` Reserve liquidity supply SPL Token account.
    { pubkey: reserveLiquiditySupply, isSigner: false, isWritable: true },
    ///   9. `[signer]` Obligation owner
    { pubkey: obligationOwner, isSigner: true, isWritable: false },
    ///   10 `[signer]` User transfer authority ($authority).
    { pubkey: transferAuthority, isSigner: true, isWritable: false },
    ///   11. `[]` Clock sysvar.
    { pubkey: SYSVAR_CLOCK_PUBKEY, isSigner: false, isWritable: false },
    ///   12. `[]` Token program id.
    { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
  ];

  return new TransactionInstruction({
    keys,
    programId: LENDING_PROGRAM_ID,
    data,
  });
};
