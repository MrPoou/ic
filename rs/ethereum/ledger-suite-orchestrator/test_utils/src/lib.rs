use crate::flow::AddErc20TokenFlow;
use crate::metrics::MetricsAssert;
use candid::{Decode, Encode, Nat, Principal};
use ic_base_types::CanisterId;
use ic_ledger_suite_orchestrator::candid::{
    AddErc20Arg, CyclesManagement, Erc20Contract, InitArg, LedgerInitArg, ManagedCanisterIds,
    OrchestratorArg, OrchestratorInfo,
};
use ic_ledger_suite_orchestrator::state::{IndexWasm, LedgerWasm, WasmHash};
use ic_state_machine_tests::{
    CanisterStatusResultV2, Cycles, StateMachine, StateMachineBuilder, UserError, WasmResult,
};
use ic_test_utilities_load_wasm::load_wasm;
pub use icrc_ledger_types::icrc::generic_metadata_value::MetadataValue as LedgerMetadataValue;
pub use icrc_ledger_types::icrc1::account::Account as LedgerAccount;
use std::sync::Arc;

pub mod arbitrary;
pub mod flow;
pub mod metrics;

const MAX_TICKS: usize = 10;
const GIT_COMMIT_HASH: &str = "6a8e5fca2c6b4e12966638c444e994e204b42989";
pub const CKERC20_TRANSFER_FEE: u64 = 4_000; //0.004 USD for ckUSDC/ckUSDT

pub const NNS_ROOT_PRINCIPAL: Principal = Principal::from_slice(&[0_u8]);

pub struct LedgerSuiteOrchestrator {
    pub env: Arc<StateMachine>,
    pub ledger_suite_orchestrator_id: CanisterId,
    pub embedded_ledger_wasm_hash: WasmHash,
    pub embedded_index_wasm_hash: WasmHash,
}

impl Default for LedgerSuiteOrchestrator {
    fn default() -> Self {
        Self::new(Arc::new(new_state_machine()), default_init_arg())
    }
}

impl AsRef<StateMachine> for LedgerSuiteOrchestrator {
    fn as_ref(&self) -> &StateMachine {
        &self.env
    }
}

impl LedgerSuiteOrchestrator {
    pub fn with_cycles_management(cycles_management: CyclesManagement) -> Self {
        let init_arg = InitArg {
            cycles_management: Some(cycles_management),
            ..default_init_arg()
        };
        Self::new(Arc::new(new_state_machine()), init_arg)
    }
    pub fn new(env: Arc<StateMachine>, init_arg: InitArg) -> Self {
        let ledger_suite_orchestrator_id =
            env.create_canister_with_cycles(None, Cycles::new(u128::MAX), None);
        install_ledger_orchestrator(&env, ledger_suite_orchestrator_id, init_arg);
        Self {
            env,
            ledger_suite_orchestrator_id,
            embedded_ledger_wasm_hash: ledger_wasm().hash().clone(),
            embedded_index_wasm_hash: index_wasm().hash().clone(),
        }
    }

    fn upgrade_ledger_suite_orchestrator_expecting_ok(self, upgrade_arg: &OrchestratorArg) -> Self {
        self.upgrade_ledger_suite_orchestrator(upgrade_arg)
            .expect("Failed to upgrade ledger suite orchestrator");
        self
    }

    pub fn upgrade_ledger_suite_orchestrator(
        &self,
        upgrade_arg: &OrchestratorArg,
    ) -> Result<(), UserError> {
        self.env.tick(); //tick before upgrade to finish current timers which are reset afterwards
        self.env.upgrade_canister(
            self.ledger_suite_orchestrator_id,
            ledger_suite_orchestrator_wasm(),
            Encode!(upgrade_arg).unwrap(),
        )
    }

    pub fn add_erc20_token(self, params: AddErc20Arg) -> AddErc20TokenFlow {
        let setup = self.upgrade_ledger_suite_orchestrator_expecting_ok(
            &OrchestratorArg::AddErc20Arg(params.clone()),
        );
        AddErc20TokenFlow { setup, params }
    }

    pub fn call_orchestrator_canister_ids(
        &self,
        contract: &Erc20Contract,
    ) -> Option<ManagedCanisterIds> {
        Decode!(
            &assert_reply(
                self.env
                    .execute_ingress(
                        self.ledger_suite_orchestrator_id,
                        "canister_ids",
                        Encode!(contract).unwrap()
                    )
                    .expect("failed to execute token transfer")
            ),
            Option<ManagedCanisterIds>
        )
        .unwrap()
    }

    pub fn advance_time_for_cycles_top_up(&self) {
        self.env
            .advance_time(std::time::Duration::from_secs(60 * 60 + 1));
        self.env.tick();
        self.env.tick();
        self.env.tick();
        self.env.tick();
        self.env.tick();
        self.env.tick();
    }

    pub fn canister_status_of(&self, controlled_canister_id: CanisterId) -> CanisterStatusResultV2 {
        self.env
            .canister_status_as(
                self.ledger_suite_orchestrator_id.into(),
                controlled_canister_id,
            )
            .unwrap()
            .unwrap()
    }

    pub fn get_orchestrator_info(&self) -> OrchestratorInfo {
        Decode!(
            &assert_reply(
                self.env
                    .query(
                        self.ledger_suite_orchestrator_id,
                        "get_orchestrator_info",
                        Encode!().unwrap()
                    )
                    .unwrap()
            ),
            OrchestratorInfo
        )
        .unwrap()
    }

    pub fn check_metrics(self) -> MetricsAssert<Self> {
        let canister_id = self.ledger_suite_orchestrator_id;
        MetricsAssert::from_querying_metrics(self, canister_id)
    }
}

fn default_init_arg() -> InitArg {
    InitArg {
        more_controller_ids: vec![NNS_ROOT_PRINCIPAL],
        minter_id: None,
        cycles_management: None,
    }
}

pub fn new_state_machine() -> StateMachine {
    StateMachineBuilder::new()
        .with_default_canister_range()
        .build()
}

fn install_ledger_orchestrator(
    env: &StateMachine,
    ledger_suite_orchestrator_id: CanisterId,
    init_arg: InitArg,
) {
    env.install_existing_canister(
        ledger_suite_orchestrator_id,
        ledger_suite_orchestrator_wasm(),
        Encode!(&OrchestratorArg::InitArg(init_arg)).unwrap(),
    )
    .unwrap();
}

fn ledger_suite_orchestrator_wasm() -> Vec<u8> {
    load_wasm(
        std::env::var("CARGO_MANIFEST_DIR").unwrap(),
        "ledger_suite_orchestrator",
        &[],
    )
}

fn ledger_wasm() -> LedgerWasm {
    LedgerWasm::from(load_wasm(
        std::env::var("CARGO_MANIFEST_DIR").unwrap(),
        "ledger_canister",
        &[],
    ))
}

fn index_wasm() -> IndexWasm {
    IndexWasm::from(load_wasm(
        std::env::var("CARGO_MANIFEST_DIR").unwrap(),
        "index_canister",
        &[],
    ))
}

pub fn supported_erc20_tokens(
    minter: Principal,
    ledger_compressed_wasm_hash: WasmHash,
    index_compressed_wasm_hash: WasmHash,
) -> Vec<AddErc20Arg> {
    vec![
        usdc(
            minter,
            ledger_compressed_wasm_hash.clone(),
            index_compressed_wasm_hash.clone(),
        ),
        usdt(
            minter,
            ledger_compressed_wasm_hash,
            index_compressed_wasm_hash,
        ),
    ]
}

pub fn usdc(
    minter: Principal,
    ledger_compressed_wasm_hash: WasmHash,
    index_compressed_wasm_hash: WasmHash,
) -> AddErc20Arg {
    AddErc20Arg {
        contract: usdc_erc20_contract(),
        ledger_init_arg: ledger_init_arg(minter, "Chain-Key USD Coin", "ckUSDC"),
        git_commit_hash: GIT_COMMIT_HASH.to_string(),
        ledger_compressed_wasm_hash: ledger_compressed_wasm_hash.to_string(),
        index_compressed_wasm_hash: index_compressed_wasm_hash.to_string(),
    }
}

pub fn usdc_erc20_contract() -> Erc20Contract {
    Erc20Contract {
        chain_id: Nat::from(1_u8),
        address: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string(),
    }
}

pub fn usdt(
    minter: Principal,
    ledger_compressed_wasm_hash: WasmHash,
    index_compressed_wasm_hash: WasmHash,
) -> AddErc20Arg {
    AddErc20Arg {
        contract: Erc20Contract {
            chain_id: Nat::from(1_u8),
            address: "0xdAC17F958D2ee523a2206206994597C13D831ec7".to_string(),
        },
        ledger_init_arg: ledger_init_arg(minter, "Chain-Key Tether USD", "ckUSDT"),
        git_commit_hash: GIT_COMMIT_HASH.to_string(),
        ledger_compressed_wasm_hash: ledger_compressed_wasm_hash.to_string(),
        index_compressed_wasm_hash: index_compressed_wasm_hash.to_string(),
    }
}

fn ledger_init_arg<U: Into<String>, V: Into<String>>(
    minter: Principal,
    token_name: U,
    token_symbol: V,
) -> LedgerInitArg {
    LedgerInitArg {
        minting_account: LedgerAccount {
            owner: minter,
            subaccount: None,
        },
        fee_collector_account: None,
        initial_balances: vec![],
        transfer_fee: CKERC20_TRANSFER_FEE.into(),
        decimals: None,
        token_name: token_name.into(),
        token_symbol: token_symbol.into(),
        token_logo: "".to_string(),
        max_memo_length: Some(80),
        feature_flags: None,
        maximum_number_of_accounts: None,
        accounts_overflow_trim_quantity: None,
    }
}

pub fn assert_reply(result: WasmResult) -> Vec<u8> {
    match result {
        WasmResult::Reply(bytes) => bytes,
        WasmResult::Reject(reject) => {
            panic!("Expected a successful reply, got a reject: {}", reject)
        }
    }
}
