/**
 * Temporary test to showcase that reserve upgrades work with CLI.
 * We'll delete this once all reserves are upgraded.
 *
 * $ anchor test --provider.cluster localnet --detach
 */

import * as anchor from "@coral-xyz/anchor";
import { PublicKey } from "@solana/web3.js";
import { expect } from "chai";
import { exec } from "node:child_process";

describe("liquidity mining", () => {
  // Configure the client to use the local cluster.
  anchor.setProvider(anchor.AnchorProvider.env());

  it("Upgrades reserves to 2.1.0 via CLI", async () => {
    // There's an ix that upgrades all program reserves to 2.1.0.
    // This ix is invocable via our CLI.
    // In this test case for comfort and more test coverage we invoke the CLI
    // command rather than crafting the ix ourselves.

    // We check this reserve before & after the upgrade.
    const SOME_TEST_RESERVE_TO_CHECK =
      "BgxfHJDzm44T7XG68MYKx7YisTjZu73tVovyZSjJMpmw";

    const rpcUrl = anchor.getProvider().connection.rpcEndpoint;

    const reserveBefore = await anchor
      .getProvider()
      .connection.getAccountInfo(new PublicKey(SOME_TEST_RESERVE_TO_CHECK));

    expect(reserveBefore.data.length).to.eq(619); // old version data length
    const expectedRentBefore = await anchor
      .getProvider()
      .connection.getMinimumBalanceForRentExemption(reserveBefore.data.length);
    // some reserves have more rent
    expect(reserveBefore.lamports).to.be.greaterThanOrEqual(expectedRentBefore);

    const command = `cargo run --quiet --bin solend-cli -- --url ${rpcUrl} upgrade-all-reserves`;
    console.log(`\$ ${command}`);
    const cliProcess = exec(command);

    // let us observe progress
    cliProcess.stderr.setEncoding("utf8");
    cliProcess.stderr.pipe(process.stderr);

    console.log("Waiting for command to finish...");
    const exitCode = await new Promise<number>((resolve) =>
      cliProcess.on("exit", (code) => resolve(code))
    );

    if (exitCode !== 0) {
      cliProcess.stdout.setEncoding("utf8");
      console.log("CLI stdout", cliProcess.stdout.read());

      throw new Error(`Command failed with exit code ${exitCode}`);
    }

    const reserveAfter = await anchor
      .getProvider()
      .connection.getAccountInfo(new PublicKey(SOME_TEST_RESERVE_TO_CHECK));

    expect(reserveAfter.data.length).to.eq(5451); // new version data length
    const expectedRentAfter = await anchor
      .getProvider()
      .connection.getMinimumBalanceForRentExemption(reserveAfter.data.length);
    expect(reserveAfter.lamports).to.be.greaterThanOrEqual(expectedRentAfter);
  });
});
