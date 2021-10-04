import { TOKEN_PROGRAM_ID } from '@solana/spl-token';
import { PublicKey, SYSVAR_CLOCK_PUBKEY, TransactionInstruction } from '@solana/web3.js';
import { struct, u8 } from 'buffer-layout';
import { LENDING_PROGRAM_ID } from '../constants';
import { u64 } from '../util';
import { LendingInstruction } from './instruction';

interface Data {
    instruction: number;
    liquidityAmount: bigint;
}

const DataLayout = struct<Data>([u8('instruction'), u64('liquidityAmount')]);

export const borrowFlashLoanInstruction = (
    liquidityAmount: number | bigint,
    sourceLiquidity: PublicKey,
    destinationLiquidity: PublicKey,
    liquidityReserve: PublicKey,
    flashLoanFeeReceiver: PublicKey,
    hostFeeReceiver: PublicKey,
    lendingMarket: PublicKey,
    lendingMarketAuthority: PublicKey,
    flashLoanProgram: PublicKey,
    transferAuthority: PublicKey
): TransactionInstruction => {
    const data = Buffer.alloc(DataLayout.span);
    DataLayout.encode(
        {
            instruction: LendingInstruction.FlashLoan,
            liquidityAmount: BigInt(liquidityAmount),
        },
        data
    );

    const keys = [
        ///   0. `[writable]` Source liquidity token account.
        { pubkey: sourceLiquidity, isSigner: false, isWritable: true },
        ///   1. `[writable]` Destination liquidity token account.
        { pubkey: destinationLiquidity, isSigner: false, isWritable: true },
        ///   2. `[writable]` Reserve account.
        { pubkey: liquidityReserve, isSigner: false, isWritable: true },
        ///   3. `[writable]` Flash loan fee receiver account.
        { pubkey: flashLoanFeeReceiver, isSigner: false, isWritable: true },
        ///   4. `[writable]` Host fee receiver.
        { pubkey: hostFeeReceiver, isSigner: false, isWritable: true },
        ///   5. `[]` Lending market account.
        { pubkey: lendingMarket, isSigner: false, isWritable: false },
        ///   6. `[]` Derived lending market authority.
        { pubkey: lendingMarketAuthority, isSigner: false, isWritable: false },
        ///   7. `[]` Token program id.
        { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
        ///   8. `[]` Flash loan receiver program id.
        { pubkey: flashLoanProgram, isSigner: false, isWritable: false },
        ///   9. `[signer]` user transfer authority.
        { pubkey: transferAuthority, isSigner: true, isWritable: false },
    ];

    return new TransactionInstruction({
        keys,
        programId: LENDING_PROGRAM_ID,
        data,
    });
};
