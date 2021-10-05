import {Blockchain} from './blockchain';
import {assert} from './util';

async function main() {
  // --------------------------------------- init

  const bc = new Blockchain();
  await bc.getConnection();
  await bc.initLendingMarket();
  await bc.initReserve(bc.tokenA, 100, 40);
  await bc.initReserve(bc.tokenB, 10, 4);
  await bc.initObligation();
  await bc.calcAndPrintMetrics();

  // check user lost tokens
  assert(bc.metrics.tokenAUserBalance.value.uiAmount == 100 - 40);
  assert(bc.metrics.tokenBUserBalance.value.uiAmount == 10 - 4);
  // check protocol gained tokens
  assert(bc.metrics.tokenAProtocolBalance.value.uiAmount == 40);
  assert(bc.metrics.tokenBProtocolBalance.value.uiAmount == 4);
  // check user was issued LP tokens in return
  assert(bc.metrics.tokenALPUserBalance.value.uiAmount == 40);
  assert(bc.metrics.tokenBLPUserBalance.value.uiAmount == 4);
  // check total liquidity available
  // @ts-ignore
  assert(bc.metrics.reserveAState.data.liquidity.availableAmount == 40n);
  // @ts-ignore
  assert(bc.metrics.reserveBState.data.liquidity.availableAmount == 4n);

  // --------------------------------------- deposit into reserve + obligation

  await bc.depositReserveLiquidityAndObligationCollateral(bc.tokenA, 20);
  await bc.refreshOblig();
  await bc.calcAndPrintMetrics();

  // check changes in balances add up
  assert(bc.metrics.tokenAUserBalance.value.uiAmount == 100 - 40 - 20);
  assert(bc.metrics.tokenAProtocolBalance.value.uiAmount == 40 + 20);
  // check user deposited some of their LP tokens
  assert(bc.metrics.tokenALPUserBalance.value.uiAmount == 40);
  assert(bc.metrics.tokenALPProtocolBalance.value.uiAmount == 20);
  // check obligation no longer emptry (not checking for specific numbers due to price fluctuation)
  assert(bc.metrics.obligState.data.depositedValue.toNumber() > 0);
  assert(bc.metrics.obligState.data.allowedBorrowValue.toNumber() > 0);
  assert(bc.metrics.obligState.data.unhealthyBorrowValue.toNumber() > 0);

  // --------------------------------------- withdraw from obligation + reserve

  await bc.withdrawObligationCollateralAndRedeemReserveCollateral(bc.tokenA, 10);
  await bc.refreshOblig();
  await bc.calcAndPrintMetrics();

  // check changes in balances add up
  assert(bc.metrics.tokenAUserBalance.value.uiAmount == 100 - 40 - 20 + 10);
  assert(bc.metrics.tokenAProtocolBalance.value.uiAmount == 40 + 20 - 10);
  // check user deposited some of their LP tokens
  assert(bc.metrics.tokenALPUserBalance.value.uiAmount == 40);
  assert(bc.metrics.tokenALPProtocolBalance.value.uiAmount == 10);

  console.log('All tests passed!');
}

main()
  .catch(err => {
    console.error(err);
    process.exit(-1);
  })
  .then(() => process.exit());
