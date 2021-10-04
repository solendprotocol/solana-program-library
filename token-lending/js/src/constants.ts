import { PublicKey } from '@solana/web3.js';
import BigNumber from 'bignumber.js';

// all 3 below are DEVNET
// export const LENDING_PROGRAM_ID = new PublicKey('ALend7Ketfx5bxh6ghsCDXAoDrhvEmsXT3cynB6aPLgx');
export const LENDING_PROGRAM_ID = new PublicKey('jj1rmvY2oKSE3GvukuzepoCeMo1X5M4iiKZmqNHGb41');
// export const LENDING_PROGRAM_ID = new PublicKey('8qdJZwaeDUPFGdbriVhhHhyNPFvE8tYjvYL7pBWS9pmM');
export const ORACLE_PROGRAM_ID = new PublicKey('gSbePebfvPy7tRqimPoVecS2UsBvYv46ynrzWocc92s');
export const SWITCHBOARD_PROGRAM_ID = new PublicKey('7azgmy1pFXHikv36q1zZASvFq5vFa39TT9NweVugKKTU');

/** @internal */
export const WAD = new BigNumber('1e+18');

/** @internal */
export const WAD_BigInt = BigInt(WAD.toString());

