use super::*;

use pyth_sdk_solana::state::PROD_ACCT_SIZE;
use solana_program::{
    clock::Clock,
    instruction::Instruction,
    program_pack::{IsInitialized, Pack},
    pubkey::Pubkey,
    rent::Rent,
    system_instruction, sysvar,
};
use solana_sdk::{
    commitment_config::CommitmentLevel,
    compute_budget::ComputeBudgetInstruction,
    signature::{Keypair, Signer},
    system_instruction::create_account,
    transaction::Transaction,
};
use solend_program::{
    instruction::{
        deposit_obligation_collateral, deposit_reserve_liquidity, init_lending_market,
        init_reserve, redeem_reserve_collateral,
    },
    processor::process_instruction,
    state::{LendingMarket, Reserve, ReserveConfig},
};

use spl_token::state::{Account as Token, Mint};
use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
};

use super::mock_pyth::{init, mock_pyth_program, set_price};

pub struct SolendProgramTest {
    pub context: ProgramTestContext,
    rent: Rent,

    // authority of all mints
    authority: Keypair,

    mints: HashMap<Pubkey, Option<Oracle>>,
}

#[derive(Debug, Clone, Copy)]
struct Oracle {
    pyth_product_pubkey: Pubkey,
    pyth_price_pubkey: Pubkey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info<T> {
    pub pubkey: Pubkey,
    pub account: T,
}

impl SolendProgramTest {
    pub async fn start_new() -> Self {
        let mut test = ProgramTest::new(
            "solend_program",
            solend_program::id(),
            processor!(process_instruction),
        );

        test.prefer_bpf(false);
        test.add_program(
            "mock_pyth",
            mock_pyth_program::id(),
            processor!(mock_pyth::process_instruction),
        );

        let authority = Keypair::new();

        add_mint(&mut test, usdc_mint::id(), 6, authority.pubkey());
        add_mint(&mut test, wsol_mint::id(), 9, authority.pubkey());

        let mut context = test.start_with_context().await;
        let rent = context.banks_client.get_rent().await.unwrap();

        SolendProgramTest {
            context,
            rent,
            authority,
            mints: HashMap::from([(usdc_mint::id(), None), (wsol_mint::id(), None)]),
        }
    }

    pub async fn process_transaction(
        &mut self,
        instructions: &[Instruction],
        signers: Option<&[&Keypair]>,
    ) -> Result<(), BanksClientError> {
        let mut transaction =
            Transaction::new_with_payer(instructions, Some(&self.context.payer.pubkey()));

        let mut all_signers = vec![&self.context.payer];

        if let Some(signers) = signers {
            all_signers.extend_from_slice(signers);
        }

        // This fails when warping is involved - https://gitmemory.com/issue/solana-labs/solana/18201/868325078
        // let recent_blockhash = self.context.banks_client.get_recent_blockhash().await.unwrap();

        transaction.sign(&all_signers, self.context.last_blockhash);

        self.context
            .banks_client
            .process_transaction_with_commitment(transaction, CommitmentLevel::Finalized)
            .await
    }

    pub async fn load_account<T: Pack + IsInitialized>(&mut self, acc_pk: Pubkey) -> Info<T> {
        let acc = self
            .context
            .banks_client
            .get_account(acc_pk)
            .await
            .unwrap()
            .unwrap();

        Info {
            pubkey: acc_pk,
            account: T::unpack(&acc.data).unwrap(),
        }
    }

    pub async fn get_bincode_account<T: serde::de::DeserializeOwned>(
        &mut self,
        address: &Pubkey,
    ) -> T {
        self.context
            .banks_client
            .get_account(*address)
            .await
            .unwrap()
            .map(|a| bincode::deserialize::<T>(&a.data).unwrap())
            .unwrap_or_else(|| panic!("GET-TEST-ACCOUNT-ERROR"))
    }

    #[allow(dead_code)]
    pub async fn get_clock(&mut self) -> Clock {
        self.get_bincode_account::<Clock>(&sysvar::clock::id())
            .await
    }

    /// Advances clock by x slots. note that transactions don't automatically increment the slot
    /// value in Clock, so this function must be explicitly called whenever you want time to move
    /// forward.
    pub async fn advance_clock_by_slots(&mut self, slots: u64) {
        let clock: Clock = self.get_clock().await;
        self.context.warp_to_slot(clock.slot + slots).unwrap();
    }

    pub async fn create_account(&mut self, size: usize, owner: &Pubkey) -> Pubkey {
        let keypair = Keypair::new();
        let rent = self.rent.minimum_balance(size);

        let instructions = [system_instruction::create_account(
            &self.context.payer.pubkey(),
            &keypair.pubkey(),
            rent as u64,
            size as u64,
            owner,
        )];

        self.process_transaction(&instructions, Some(&[&keypair]))
            .await
            .unwrap();

        keypair.pubkey()
    }

    pub async fn create_mint(&mut self, mint_authority: &Pubkey) -> Pubkey {
        let keypair = Keypair::new();
        let rent = self.rent.minimum_balance(Mint::LEN);

        let instructions = [
            system_instruction::create_account(
                &self.context.payer.pubkey(),
                &keypair.pubkey(),
                rent,
                Mint::LEN as u64,
                &spl_token::id(),
            ),
            spl_token::instruction::initialize_mint(
                &spl_token::id(),
                &keypair.pubkey(),
                mint_authority,
                None,
                0,
            )
            .unwrap(),
        ];

        self.process_transaction(&instructions, Some(&[&keypair]))
            .await
            .unwrap();

        keypair.pubkey()
    }

    pub async fn create_token_account(&mut self, owner: &Pubkey, mint: &Pubkey) -> Pubkey {
        let keypair = Keypair::new();
        let instructions = [
            system_instruction::create_account(
                &self.context.payer.pubkey(),
                &keypair.pubkey(),
                self.rent.minimum_balance(Token::LEN),
                spl_token::state::Account::LEN as u64,
                &spl_token::id(),
            ),
            spl_token::instruction::initialize_account(
                &spl_token::id(),
                &keypair.pubkey(),
                mint,
                owner,
            )
            .unwrap(),
        ];

        self.process_transaction(&instructions, Some(&[&keypair]))
            .await
            .unwrap();

        keypair.pubkey()
    }

    pub async fn mint_to(&mut self, mint: &Pubkey, dst: &Pubkey, amount: u64) {
        assert!(self.mints.contains_key(mint));

        let instructions = [spl_token::instruction::mint_to(
            &spl_token::id(),
            mint,
            dst,
            &self.authority.pubkey(),
            &[],
            amount,
        )
        .unwrap()];

        let authority = Keypair::from_bytes(&self.authority.to_bytes()).unwrap(); // hack
        self.process_transaction(&instructions, Some(&[&authority]))
            .await
            .unwrap();
    }

    // wrappers around solend instructions. these should be used to test logic things (eg you can't
    // borrow more than the borrow limit, but these methods can't be used to test account-level
    // security of an instruction (eg what happens if im not the lending market owner but i try to
    // add a reserve anyways).

    pub async fn init_lending_market(&mut self, owner: &User) -> Info<LendingMarket> {
        let lending_market_key = Keypair::new();
        let payer = self.context.payer.pubkey();
        let lamports = Rent::minimum_balance(&self.rent, LendingMarket::LEN);

        self.process_transaction(
            &[
                create_account(
                    &payer,
                    &lending_market_key.pubkey(),
                    lamports,
                    LendingMarket::LEN as u64,
                    &solend_program::id(),
                ),
                init_lending_market(
                    solend_program::id(),
                    owner.keypair.pubkey(),
                    QUOTE_CURRENCY,
                    lending_market_key.pubkey(),
                    mock_pyth_program::id(),
                    mock_pyth_program::id(), // TODO suspicious
                ),
            ],
            Some(&[&lending_market_key]),
        )
        .await
        .unwrap();

        self.load_account::<LendingMarket>(lending_market_key.pubkey())
            .await
    }

    pub async fn init_pyth_feed(&mut self, mint: &Pubkey) {
        let pyth_price_pubkey = self.create_account(3312, &mock_pyth_program::id()).await;
        let pyth_product_pubkey = self
            .create_account(PROD_ACCT_SIZE, &mock_pyth_program::id())
            .await;

        self.process_transaction(
            &[init(
                mock_pyth_program::id(),
                pyth_price_pubkey,
                pyth_product_pubkey,
            )],
            None,
        )
        .await
        .unwrap();

        self.mints.insert(
            *mint,
            Some(Oracle {
                pyth_product_pubkey,
                pyth_price_pubkey,
            }),
        );
    }

    pub async fn set_price(&mut self, mint: &Pubkey, price: PriceArgs) {
        let oracle = self.mints.get(mint).unwrap().unwrap();
        self.process_transaction(
            &[set_price(
                mock_pyth_program::id(),
                oracle.pyth_price_pubkey,
                price.price,
                price.conf,
                price.expo,
            )],
            None,
        )
        .await
        .unwrap();
    }

    pub async fn init_reserve(
        &mut self,
        lending_market: &Info<LendingMarket>,
        lending_market_owner: &User,
        mint: &Pubkey,
        reserve_config: &ReserveConfig,
        liquidity_amount: u64,
    ) -> Info<Reserve> {
        // let payer = self.context.payer.pubkey();
        // let authority = Keypair::from_bytes(&self.authority.to_bytes()).unwrap(); // hack

        let destination_collateral_pubkey = self.create_account(Token::LEN, &spl_token::id()).await;
        let reserve_pubkey = self
            .create_account(Reserve::LEN, &solend_program::id())
            .await;
        let reserve_liquidity_supply_pubkey =
            self.create_account(Token::LEN, &spl_token::id()).await;

        let reserve_liquidity_fee_receiver =
            self.create_account(Token::LEN, &spl_token::id()).await;

        let reserve_collateral_mint_pubkey = self.create_account(Mint::LEN, &spl_token::id()).await;
        let reserve_collateral_supply_pubkey =
            self.create_account(Token::LEN, &spl_token::id()).await;

        let oracle = self.mints.get(mint).unwrap().unwrap();

        self.process_transaction(
            &[
                ComputeBudgetInstruction::set_compute_unit_limit(70_000),
                init_reserve(
                    solend_program::id(),
                    liquidity_amount,
                    ReserveConfig {
                        fee_receiver: reserve_liquidity_fee_receiver,
                        ..*reserve_config
                    },
                    lending_market_owner.get_account(mint).await.unwrap(),
                    destination_collateral_pubkey,
                    reserve_pubkey,
                    *mint,
                    reserve_liquidity_supply_pubkey,
                    reserve_collateral_mint_pubkey,
                    reserve_collateral_supply_pubkey,
                    oracle.pyth_product_pubkey,
                    oracle.pyth_price_pubkey,
                    Pubkey::from_str("nu11111111111111111111111111111111111111111").unwrap(),
                    lending_market.pubkey,
                    lending_market_owner.keypair.pubkey(),
                    lending_market_owner.keypair.pubkey(),
                ),
            ],
            Some(&[&lending_market_owner.keypair]),
        )
        .await
        .unwrap();

        self.load_account::<Reserve>(reserve_pubkey).await
    }
}

/// 1 User holds many token accounts
#[derive(Debug)]
pub struct User {
    pub keypair: Keypair,
    pub token_accounts: Vec<Info<Token>>,
}

impl User {
    pub fn new_with_keypair(keypair: Keypair) -> Self {
        User {
            keypair,
            token_accounts: Vec::new(),
        }
    }

    /// Creates a user with specified token accounts and balances. This function only works if the
    /// SolendProgramTest object owns the mint authorities. eg this won't work for native SOL.
    pub async fn new_with_balances(
        test: &mut SolendProgramTest,
        mints_and_balances: &[(&Pubkey, u64)],
    ) -> Self {
        let mut user = User {
            keypair: Keypair::new(),
            token_accounts: Vec::new(),
        };

        for (mint, balance) in mints_and_balances {
            let token_account = user.create_token_account(mint, test).await;
            if *balance > 0 {
                test.mint_to(mint, &token_account.pubkey, *balance).await;
            }
        }

        user
    }

    pub async fn get_account(&self, mint: &Pubkey) -> Option<Pubkey> {
        self.token_accounts.iter().find_map(|ta| {
            if ta.account.mint == *mint {
                Some(ta.pubkey)
            } else {
                None
            }
        })
    }

    pub async fn create_token_account(
        &mut self,
        mint: &Pubkey,
        test: &mut SolendProgramTest,
    ) -> Info<Token> {
        match self
            .token_accounts
            .iter()
            .find(|ta| ta.account.mint == *mint)
        {
            None => {
                let pubkey = test
                    .create_token_account(&self.keypair.pubkey(), mint)
                    .await;
                let account = test.load_account::<Token>(pubkey).await;

                self.token_accounts.push(account.clone());

                account
            }
            Some(_) => panic!("Token account already exists!"),
        }
    }

    pub async fn transfer(
        &self,
        mint: &Pubkey,
        destination_pubkey: Pubkey,
        amount: u64,
        test: &mut SolendProgramTest,
    ) {
        let instruction = [spl_token::instruction::transfer(
            &spl_token::id(),
            &self.get_account(mint).await.unwrap(),
            &destination_pubkey,
            &self.keypair.pubkey(),
            &[],
            amount,
        )
        .unwrap()];

        test.process_transaction(&instruction, Some(&[&self.keypair]))
            .await
            .unwrap();
    }
}

pub struct PriceArgs {
    pub price: i64,
    pub conf: u64,
    pub expo: i32,
}

impl Info<LendingMarket> {
    pub async fn deposit(
        &self,
        test: &mut SolendProgramTest,
        reserve: &Info<Reserve>,
        user: &User,
        liquidity_amount: u64,
    ) -> Result<(), BanksClientError> {
        let instructions = [deposit_reserve_liquidity(
            solend_program::id(),
            liquidity_amount,
            user.get_account(&reserve.account.liquidity.mint_pubkey)
                .await
                .unwrap(),
            user.get_account(&reserve.account.collateral.mint_pubkey)
                .await
                .unwrap(),
            reserve.pubkey,
            reserve.account.liquidity.supply_pubkey,
            reserve.account.collateral.mint_pubkey,
            self.pubkey,
            user.keypair.pubkey(),
        )];

        test.process_transaction(&instructions, Some(&[&user.keypair]))
            .await
    }

    pub async fn redeem(
        &self,
        test: &mut SolendProgramTest,
        reserve: &Info<Reserve>,
        user: &User,
        collateral_amount: u64,
    ) -> Result<(), BanksClientError> {
        let instructions = [redeem_reserve_collateral(
            solend_program::id(),
            collateral_amount,
            user.get_account(&reserve.account.collateral.mint_pubkey)
                .await
                .unwrap(),
            user.get_account(&reserve.account.liquidity.mint_pubkey)
                .await
                .unwrap(),
            reserve.pubkey,
            reserve.account.collateral.mint_pubkey,
            reserve.account.liquidity.supply_pubkey,
            self.pubkey,
            user.keypair.pubkey(),
        )];

        test.process_transaction(&instructions, Some(&[&user.keypair]))
            .await
    }

    pub async fn init_obligation(
        &self,
        test: &mut SolendProgramTest,
        obligation_keypair: Keypair,
        user: &User,
    ) -> Result<Info<Obligation>, BanksClientError> {
        let instructions = [
            system_instruction::create_account(
                &test.context.payer.pubkey(),
                &obligation_keypair.pubkey(),
                Rent::minimum_balance(&Rent::default(), Obligation::LEN),
                Obligation::LEN as u64,
                &solend_program::id(),
            ),
            init_obligation(
                solend_program::id(),
                obligation_keypair.pubkey(),
                self.pubkey,
                user.keypair.pubkey(),
            ),
        ];

        match test
            .process_transaction(&instructions, Some(&[&obligation_keypair, &user.keypair]))
            .await
        {
            Ok(()) => Ok(test
                .load_account::<Obligation>(obligation_keypair.pubkey())
                .await),
            Err(e) => Err(e),
        }
    }

    pub async fn deposit_obligation_collateral(
        &self,
        test: &mut SolendProgramTest,
        reserve: &Info<Reserve>,
        obligation: &Info<Obligation>,
        user: &User,
        collateral_amount: u64,
    ) -> Result<(), BanksClientError> {
        let instructions = [deposit_obligation_collateral(
            solend_program::id(),
            collateral_amount,
            user.get_account(&reserve.account.collateral.mint_pubkey)
                .await
                .unwrap(),
            reserve.account.collateral.supply_pubkey,
            reserve.pubkey,
            obligation.pubkey,
            self.pubkey,
            user.keypair.pubkey(),
            user.keypair.pubkey(),
        )];

        test.process_transaction(&instructions, Some(&[&user.keypair]))
            .await
    }

    pub async fn refresh_reserve(
        &self,
        test: &mut SolendProgramTest,
        reserve: &Info<Reserve>,
    ) -> Result<(), BanksClientError> {
        test.process_transaction(
            &[refresh_reserve(
                solend_program::id(),
                reserve.pubkey,
                reserve.account.liquidity.pyth_oracle_pubkey,
                reserve.account.liquidity.switchboard_oracle_pubkey,
            )],
            None,
        )
        .await
    }

    pub async fn build_refresh_instructions(
        &self,
        test: &mut SolendProgramTest,
        obligation: &Info<Obligation>,
        extra_reserve: Option<&Info<Reserve>>,
    ) -> Vec<Instruction> {
        let reserve_pubkeys: Vec<Pubkey> = {
            let mut r = HashSet::new();
            r.extend(
                obligation
                    .account
                    .deposits
                    .iter()
                    .map(|d| d.deposit_reserve),
            );
            r.extend(obligation.account.borrows.iter().map(|b| b.borrow_reserve));

            if let Some(reserve) = extra_reserve {
                r.insert(reserve.pubkey);
            }

            r.into_iter().collect()
        };

        let mut reserves = Vec::new();
        for pubkey in reserve_pubkeys {
            reserves.push(test.load_account::<Reserve>(pubkey).await);
        }

        let mut instructions: Vec<Instruction> = reserves
            .into_iter()
            .map(|reserve| {
                refresh_reserve(
                    solend_program::id(),
                    reserve.pubkey,
                    reserve.account.liquidity.pyth_oracle_pubkey,
                    reserve.account.liquidity.switchboard_oracle_pubkey,
                )
            })
            .collect();

        let reserve_pubkeys: Vec<Pubkey> = {
            let mut r = Vec::new();
            r.extend(
                obligation
                    .account
                    .deposits
                    .iter()
                    .map(|d| d.deposit_reserve),
            );
            r.extend(obligation.account.borrows.iter().map(|b| b.borrow_reserve));
            r
        };

        instructions.push(refresh_obligation(
            solend_program::id(),
            obligation.pubkey,
            reserve_pubkeys,
        ));

        instructions
    }

    pub async fn refresh_obligation(
        &self,
        test: &mut SolendProgramTest,
        obligation: &Info<Obligation>,
    ) -> Result<(), BanksClientError> {
        let instructions = self
            .build_refresh_instructions(test, obligation, None)
            .await;

        test.process_transaction(&instructions, None).await
    }

    pub async fn borrow_obligation_liquidity(
        &self,
        test: &mut SolendProgramTest,
        borrow_reserve: &Info<Reserve>,
        obligation: &Info<Obligation>,
        user: &User,
        host_fee_receiver_pubkey: &Pubkey,
        liquidity_amount: u64,
    ) -> Result<(), BanksClientError> {
        let mut instructions = self
            .build_refresh_instructions(test, obligation, Some(borrow_reserve))
            .await;

        instructions.push(borrow_obligation_liquidity(
            solend_program::id(),
            liquidity_amount,
            borrow_reserve.account.liquidity.supply_pubkey,
            user.get_account(&borrow_reserve.account.liquidity.mint_pubkey)
                .await
                .unwrap(),
            borrow_reserve.pubkey,
            borrow_reserve.account.config.fee_receiver,
            obligation.pubkey,
            self.pubkey,
            user.keypair.pubkey(),
            Some(*host_fee_receiver_pubkey),
        ));

        test.process_transaction(&instructions, Some(&[&user.keypair]))
            .await
    }
}

/// Track token balance changes across transactions.
pub struct BalanceChecker {
    token_accounts: Vec<Info<Token>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BalanceChange {
    pub token_account: Pubkey,
    pub mint: Pubkey,
    pub diff: i128,
}

impl BalanceChecker {
    pub async fn start(test: &mut SolendProgramTest, objs: &[&dyn GetTokenAccounts]) -> Self {
        let mut refreshed_accounts = Vec::new();
        for obj in objs {
            for pubkey in obj.get_token_accounts() {
                let refreshed_account = test.load_account::<Token>(pubkey).await;
                refreshed_accounts.push(refreshed_account);
            }
        }

        BalanceChecker {
            token_accounts: refreshed_accounts,
        }
    }

    pub async fn find_balance_changes(
        &self,
        test: &mut SolendProgramTest,
    ) -> HashSet<BalanceChange> {
        let mut balance_changes = HashSet::new();
        for token_account in &self.token_accounts {
            let refreshed_token_account = test.load_account::<Token>(token_account.pubkey).await;

            if refreshed_token_account.account.amount != token_account.account.amount {
                balance_changes.insert(BalanceChange {
                    token_account: token_account.pubkey,
                    mint: token_account.account.mint,
                    diff: (refreshed_token_account.account.amount as i128)
                        - (token_account.account.amount as i128),
                });
            }
        }

        balance_changes
    }
}

/// trait that tracks token accounts associated with a specific struct
pub trait GetTokenAccounts {
    fn get_token_accounts(&self) -> Vec<Pubkey>;
}

impl GetTokenAccounts for User {
    fn get_token_accounts(&self) -> Vec<Pubkey> {
        self.token_accounts.iter().map(|a| a.pubkey).collect()
    }
}

impl GetTokenAccounts for Info<Reserve> {
    fn get_token_accounts(&self) -> Vec<Pubkey> {
        vec![
            self.account.liquidity.supply_pubkey,
            self.account.collateral.supply_pubkey,
            self.account.config.fee_receiver,
        ]
    }
}

pub async fn setup_world(usdc_reserve_config: &ReserveConfig, wsol_reserve_config: &ReserveConfig) -> (
    SolendProgramTest,
    Info<LendingMarket>,
    Info<Reserve>,
    Info<Reserve>,
    User,
    User,
) {
    let mut test = SolendProgramTest::start_new().await;

    let lending_market_owner = User::new_with_balances(
        &mut test,
        &[
            (&usdc_mint::id(), 1_000_000),
            (&wsol_mint::id(), LAMPORTS_TO_SOL),
        ],
    )
    .await;

    let lending_market = test.init_lending_market(&lending_market_owner).await;

    test.advance_clock_by_slots(999).await;

    test.init_pyth_feed(&usdc_mint::id()).await;
    test.set_price(
        &usdc_mint::id(),
        PriceArgs {
            price: 1,
            conf: 0,
            expo: 0,
        },
    )
    .await;

    test.init_pyth_feed(&wsol_mint::id()).await;
    test.set_price(
        &wsol_mint::id(),
        PriceArgs {
            price: 10,
            conf: 0,
            expo: 0,
        },
    )
    .await;

    let usdc_reserve = test
        .init_reserve(
            &lending_market,
            &lending_market_owner,
            &usdc_mint::id(),
            usdc_reserve_config,
            1_000_000,
        )
        .await;

    let wsol_reserve = test
        .init_reserve(
            &lending_market,
            &lending_market_owner,
            &wsol_mint::id(),
            wsol_reserve_config,
            1_000_000_000,
        )
        .await;

    let user = User::new_with_balances(
        &mut test,
        &[
            (&usdc_mint::id(), 1_000_000_000_000),             // 1M USDC
            (&usdc_reserve.account.collateral.mint_pubkey, 0), // cUSDC
            (&wsol_mint::id(), 0),
        ],
    )
    .await;

    (
        test,
        lending_market,
        usdc_reserve,
        wsol_reserve,
        lending_market_owner,
        user,
    )
}