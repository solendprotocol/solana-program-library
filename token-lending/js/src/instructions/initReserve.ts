import { TOKEN_PROGRAM_ID } from "@solana/spl-token";
import {
  PublicKey,
  SYSVAR_CLOCK_PUBKEY,
  SYSVAR_RENT_PUBKEY,
  TransactionInstruction,
} from "@solana/web3.js";
import { struct, u8 } from "buffer-layout";
import { LENDING_PROGRAM_ID } from "../constants";
import { ReserveConfig, ReserveConfigLayout } from "../state";
import { u64 } from "../util";
import { LendingInstruction } from "./instruction";

interface Data {
  instruction: number;
  liquidityAmount: bigint;
  config: ReserveConfig;
}

const DataLayout = struct<Data>([
  u8("instruction"),
  u64("liquidityAmount"),
  ReserveConfigLayout,
]);

export const initReserveInstruction = (
  liquidityAmount: number | bigint,
  config: ReserveConfig,
  sourceLiquidity: PublicKey,
  destinationCollateral: PublicKey,
  reserve: PublicKey,
  liquidityMint: PublicKey,
  liquiditySupply: PublicKey,
  liquidityFeeReceiver: PublicKey,
  pythProduct: PublicKey,
  pythPrice: PublicKey,
  collateralMint: PublicKey,
  collateralSupply: PublicKey,
  lendingMarket: PublicKey,
  lendingMarketAuthority: PublicKey,
  lendingMarketOwner: PublicKey,
  transferAuthority: PublicKey,
  switchboardFeed,
): TransactionInstruction => {
  const data = Buffer.alloc(DataLayout.span);
  DataLayout.encode(
    {
      instruction: LendingInstruction.InitReserve,
      liquidityAmount: BigInt(liquidityAmount),
      config,
    },
    data
  );

  const keys = [
    //    let source_liquidity_info = next_account_info(account_info_iter)?;
    { pubkey: sourceLiquidity, isSigner: false, isWritable: true },
    //     let destination_collateral_info = next_account_info(account_info_iter)?;
    { pubkey: destinationCollateral, isSigner: false, isWritable: true },
    //     let reserve_info = next_account_info(account_info_iter)?;
    { pubkey: reserve, isSigner: false, isWritable: true },
    //     let reserve_liquidity_mint_info = next_account_info(account_info_iter)?;
    { pubkey: liquidityMint, isSigner: false, isWritable: false },
    //     let reserve_liquidity_supply_info = next_account_info(account_info_iter)?;
    { pubkey: liquiditySupply, isSigner: false, isWritable: true },
    //     let reserve_liquidity_fee_receiver_info = next_account_info(account_info_iter)?;
    { pubkey: liquidityFeeReceiver, isSigner: false, isWritable: true },
    //     let reserve_collateral_mint_info = next_account_info(account_info_iter)?;
    { pubkey: collateralMint, isSigner: false, isWritable: true },
    //     let reserve_collateral_supply_info = next_account_info(account_info_iter)?;
    { pubkey: collateralSupply, isSigner: false, isWritable: true },
    //     let pyth_product_info = next_account_info(account_info_iter)?;
    { pubkey: pythProduct, isSigner: false, isWritable: false },
    //     let pyth_price_info = next_account_info(account_info_iter)?;
    { pubkey: pythPrice, isSigner: false, isWritable: false },
    //     todo let switchboard_feed_info = next_account_info(account_info_iter)?;
    { pubkey: switchboardFeed, isSigner: false, isWritable: false },
    //     let lending_market_info = next_account_info(account_info_iter)?;
    { pubkey: lendingMarket, isSigner: false, isWritable: true },
    //     let lending_market_authority_info = next_account_info(account_info_iter)?;
    { pubkey: lendingMarketAuthority, isSigner: false, isWritable: false },
    //     let lending_market_owner_info = next_account_info(account_info_iter)?;
    { pubkey: lendingMarketOwner, isSigner: true, isWritable: false },
    //     let user_transfer_authority_info = next_account_info(account_info_iter)?;
    { pubkey: transferAuthority, isSigner: true, isWritable: false },
    //     let clock = &Clock::from_account_info(next_account_info(account_info_iter)?)?;
    { pubkey: SYSVAR_CLOCK_PUBKEY, isSigner: false, isWritable: false },
    //     let rent_info = next_account_info(account_info_iter)?;
    //     let rent = &Rent::from_account_info(rent_info)?;
    { pubkey: SYSVAR_RENT_PUBKEY, isSigner: false, isWritable: false },
    //     let token_program_id = next_account_info(account_info_iter)?;
    { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
  ];

  return new TransactionInstruction({
    keys,
    programId: LENDING_PROGRAM_ID,
    data,
  });
};
