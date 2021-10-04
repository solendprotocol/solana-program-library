import { Connection, Keypair, LAMPORTS_PER_SOL } from '@solana/web3.js';

export function assert(condition: boolean, message?: string) {
  if (!condition) {
    console.log(Error().stack + ':main.ts');
    throw message || 'Assertion failed';
  }
}

export async function newAccountWithLamports(
  connection: Connection,
  lamports: number = 1000000,
): Promise<Keypair> {
  const account = new Keypair();
  await requestAirdrop(connection, lamports, account);
  return account;
}

export function sleep(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms));
}

async function requestAirdrop(connection: Connection, lamports: number, account: Keypair) {
  //new restriction of max 5 sol
  if (lamports > LAMPORTS_PER_SOL * 5) {
    lamports = LAMPORTS_PER_SOL * 5
  }

  let retries = 30;
  // console.log("new account is ", account);
  await connection.requestAirdrop(account.publicKey, lamports);
  for (;;) {
    // console.log('round', retries)
    await sleep(500);
    if (lamports == (await connection.getBalance(account.publicKey))) {
      console.log(`Airdrop for ${lamports / LAMPORTS_PER_SOL} SOL was successful.`)
      return account;
    }
    if (--retries <= 0) {
      break;
    }
  }
  throw new Error(`Airdrop of ${lamports} failed`);
}

export async function pause(ms: number) {
  //weird semantics - but needed to work inside jest
  //taken from https://stackoverflow.com/questions/46077176/jest-settimeout-not-pausing-test
  await new Promise(response => setTimeout(() => {
      response(0)
    }, ms)
  );
}
