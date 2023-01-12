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
use solana_program_test::*;
use solana_sdk::{
    commitment_config::CommitmentLevel,
    compute_budget::ComputeBudgetInstruction,
    signature::{Keypair, Signer},
    system_instruction::create_account,
    transaction::Transaction,
};
use solend_program::{
    instruction::{deposit_reserve_liquidity, init_lending_market, init_reserve},
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

#[derive(Debug, Clone)]
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

    pub async fn load_account<T: Pack + IsInitialized>(&mut self, acc_pk: Pubkey) -> T {
        let acc = self
            .context
            .banks_client
            .get_account(acc_pk)
            .await
            .unwrap()
            .unwrap();
        T::unpack(&acc.data).unwrap()
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
        let mut clock: Clock = self.get_clock().await;
        println!("clock slot before: {}", clock.slot);
        self.context.warp_to_slot(clock.slot + slots).unwrap();
        clock = self.get_clock().await;
        println!("clock slot after: {}", clock.slot);
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

        Info {
            pubkey: lending_market_key.pubkey(),
            account: self
                .load_account::<LendingMarket>(lending_market_key.pubkey())
                .await,
        }
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

        Info {
            pubkey: reserve_pubkey,
            account: self.load_account::<Reserve>(reserve_pubkey).await,
        }
    }
}

/// 1 User holds many token accounts
#[derive(Debug)]
pub struct User {
    pub keypair: Keypair,
    pub token_accounts: Vec<Info<Token>>,
}

impl User {
    pub fn new() -> Self {
        User {
            keypair: Keypair::new(),
            token_accounts: Vec::new(),
        }
    }

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
        let mut user = User::new();

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

                self.token_accounts.push(Info { pubkey, account });

                Info { pubkey, account }
            }
            Some(_) => panic!("Token account already exists!"),
        }
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
                refreshed_accounts.push(Info {
                    pubkey,
                    account: refreshed_account,
                });
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

            if refreshed_token_account.amount != token_account.account.amount {
                balance_changes.insert(BalanceChange {
                    token_account: token_account.pubkey,
                    mint: token_account.account.mint,
                    diff: (refreshed_token_account.amount as i128)
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
