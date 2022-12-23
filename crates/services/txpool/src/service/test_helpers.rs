use super::*;
use crate::{
    ports::BlockImport,
    MockDb,
};
use fuel_core_services::{
    stream::BoxStream,
    Service as ServiceTrait,
};
use fuel_core_types::{
    blockchain::SealedBlock,
    entities::coin::Coin,
    fuel_crypto::rand::{
        rngs::StdRng,
        SeedableRng,
    },
    fuel_tx::{
        Input,
        Transaction,
        TransactionBuilder,
        Word,
    },
    services::p2p::GossipsubMessageAcceptance,
};
use std::cell::RefCell;

type GossipedTransaction = GossipData<Transaction>;

pub struct TestContext {
    pub(crate) service: Service,
    mock_db: Box<MockDb>,
    rng: RefCell<StdRng>,
}

impl TestContext {
    pub async fn new() -> Self {
        TestContextBuilder::new().build().await
    }

    pub fn service(&self) -> &Service {
        &self.service
    }

    pub fn setup_script_tx(&self, gas_price: Word) -> Transaction {
        let (_, gas_coin) = self.setup_coin();
        TransactionBuilder::script(vec![], vec![])
            .gas_price(gas_price)
            .add_input(gas_coin)
            .finalize_as_transaction()
    }

    pub fn setup_coin(&self) -> (Coin, Input) {
        crate::test_helpers::setup_coin(&mut self.rng.borrow_mut(), Some(&self.mock_db))
    }
}

mockall::mock! {
    pub P2P {}

    #[async_trait::async_trait]
    impl PeerToPeer for P2P {
        type GossipedTransaction = GossipedTransaction;

        fn broadcast_transaction(&self, transaction: Arc<Transaction>) -> anyhow::Result<()>;

        fn gossiped_transaction_events(&self) -> BoxStream<GossipedTransaction>;

        async fn notify_gossip_transaction_validity(
            &self,
            message: &GossipedTransaction,
            validity: GossipsubMessageAcceptance,
        );
    }
}

impl MockP2P {
    pub fn new_with_txs(txs: Vec<Transaction>) -> Self {
        let mut p2p = MockP2P::default();
        p2p.expect_gossiped_transaction_events().returning(move || {
            let txs_clone = txs.clone();
            let stream = fuel_core_services::stream::unfold(txs_clone, |mut txs| async {
                let tx = txs.pop();
                if let Some(tx) = tx {
                    Some((GossipData::new(tx, vec![], vec![]), txs))
                } else {
                    core::future::pending().await
                }
            });
            Box::pin(stream)
        });
        p2p.expect_broadcast_transaction()
            .returning(move |_| Ok(()));
        p2p
    }
}

mockall::mock! {
    pub Importer {}

    impl BlockImport for Importer {
        fn block_events(&self) -> BoxStream<SealedBlock>;
    }
}

impl MockImporter {
    fn with_blocks(blocks: Vec<SealedBlock>) -> Self {
        let mut importer = MockImporter::default();
        importer.expect_block_events().returning(move || {
            let blocks = blocks.clone();
            let stream = fuel_core_services::stream::unfold(blocks, |mut blocks| async {
                let block = blocks.pop();
                if let Some(block) = block {
                    Some((block, blocks))
                } else {
                    core::future::pending().await
                }
            });
            Box::pin(stream)
        });
        importer
    }
}

pub struct TestContextBuilder {
    mock_db: MockDb,
    rng: StdRng,
    p2p: Option<MockP2P>,
    importer: Option<MockImporter>,
}

impl Default for TestContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TestContextBuilder {
    pub fn new() -> Self {
        Self {
            mock_db: MockDb::default(),
            rng: StdRng::seed_from_u64(10),
            p2p: None,
            importer: None,
        }
    }

    pub fn with_importer(&mut self, importer: MockImporter) {
        self.importer = Some(importer)
    }

    pub fn with_p2p(&mut self, p2p: MockP2P) {
        self.p2p = Some(p2p)
    }

    pub fn setup_script_tx(&mut self, gas_price: Word) -> Transaction {
        let (_, gas_coin) = self.setup_coin();
        TransactionBuilder::script(vec![], vec![])
            .gas_price(gas_price)
            .add_input(gas_coin)
            .finalize_as_transaction()
    }

    pub fn setup_coin(&mut self) -> (Coin, Input) {
        crate::test_helpers::setup_coin(&mut self.rng, Some(&self.mock_db))
    }

    pub async fn build(self) -> TestContext {
        let rng = RefCell::new(self.rng);
        let config = Config::default();
        let mock_db = self.mock_db;
        let status_tx = TxStatusChange::new(100);

        let p2p = Box::new(self.p2p.unwrap_or_else(|| MockP2P::new_with_txs(vec![])));
        let importer = Box::new(
            self.importer
                .unwrap_or_else(|| MockImporter::with_blocks(vec![])),
        );

        let mut builder = ServiceBuilder::new();
        builder
            .config(config)
            .db(Arc::new(mock_db.clone()))
            .importer(importer)
            .tx_status_sender(status_tx)
            .p2p(p2p);

        let service = builder.build().unwrap();
        service.start().unwrap();

        TestContext {
            service,
            mock_db: Box::new(mock_db),
            rng,
        }
    }
}