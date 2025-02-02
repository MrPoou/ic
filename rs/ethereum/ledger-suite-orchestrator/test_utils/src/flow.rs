use crate::metrics::MetricsAssert;
use crate::{assert_reply, LedgerAccount, LedgerMetadataValue, LedgerSuiteOrchestrator, MAX_TICKS};
use candid::{Decode, Encode, Nat, Principal};
use ic_base_types::{CanisterId, PrincipalId};
use ic_ledger_suite_orchestrator::candid::{AddErc20Arg, ManagedCanisterIds};
use ic_state_machine_tests::StateMachine;
use icrc_ledger_types::icrc1::transfer::{TransferArg, TransferError};
use icrc_ledger_types::icrc3::archive::ArchiveInfo;
use std::collections::BTreeSet;

pub struct AddErc20TokenFlow {
    pub setup: LedgerSuiteOrchestrator,
    pub params: AddErc20Arg,
}

impl AddErc20TokenFlow {
    pub fn expect_new_ledger_and_index_canisters(self) -> ManagedCanistersAssert {
        for _ in 0..MAX_TICKS {
            self.setup.env.tick();
        }

        let canister_ids = self
            .setup
            .call_orchestrator_canister_ids(&self.params.contract)
            .unwrap_or_else(|| {
                panic!(
                    "No managed canister IDs found for contract {:?}",
                    self.params.contract
                )
            });

        assert_ne!(
            canister_ids.ledger, canister_ids.index,
            "BUG: ledger and index canister IDs MUST be different"
        );

        ManagedCanistersAssert {
            setup: self.setup,
            canister_ids,
        }
    }
}

pub struct ManagedCanistersAssert {
    pub setup: LedgerSuiteOrchestrator,
    pub canister_ids: ManagedCanisterIds,
}

impl AsRef<StateMachine> for ManagedCanistersAssert {
    fn as_ref(&self) -> &StateMachine {
        self.setup.env.as_ref()
    }
}

impl ManagedCanistersAssert {
    pub fn assert_all_controlled_by(self, expected_controllers: &[Principal]) -> Self {
        for canister_id in self.all_canister_ids() {
            assert_eq!(
                self.setup
                    .canister_status_of(canister_id)
                    .settings()
                    .controllers()
                    .into_iter()
                    .map(|p| p.0)
                    .collect::<BTreeSet<_>>(),
                expected_controllers
                    .iter()
                    .copied()
                    .collect::<BTreeSet<_>>(), // convert to set to ignore order
                "BUG: unexpected controller for canister {} in managed canisters {}",
                canister_id,
                self.canister_ids
            );
        }
        self
    }

    pub fn check_metrics(self) -> MetricsAssert<Self> {
        let canister_id = self.setup.ledger_suite_orchestrator_id;
        MetricsAssert::from_querying_metrics(self, canister_id)
    }

    pub fn trigger_creation_of_archive(self) -> Self {
        const ARCHIVE_TRIGGER_THRESHOLD: u64 = 2_000;

        let archive_ids_before: BTreeSet<_> = self
            .call_ledger_archives()
            .into_iter()
            .map(|info| info.canister_id)
            .collect();

        for _i in 0..ARCHIVE_TRIGGER_THRESHOLD {
            let from = Principal::anonymous();
            let to = Principal::management_canister();
            self.call_ledger_icrc1_transfer(
                from,
                &TransferArg {
                    from_subaccount: None,
                    to: to.into(),
                    fee: None,
                    created_at_time: None,
                    memo: None,
                    amount: Nat::from(1_u8),
                },
            )
            .expect("BUG: fail to make a transfer to trigger archive creation");
        }

        self.setup.env.run_until_completion(/*max_ticks=*/ 10);

        let archive_ids_after: BTreeSet<_> = self
            .call_ledger_archives()
            .into_iter()
            .map(|info| info.canister_id)
            .collect();
        assert_eq!(
            archive_ids_after.len(),
            archive_ids_before.len() + 1,
            "BUG: expected one more archive canister"
        );
        assert!(archive_ids_before.is_subset(&archive_ids_after));

        Self {
            setup: self.setup,
            canister_ids: ManagedCanisterIds {
                ledger: self.canister_ids.ledger,
                index: self.canister_ids.index,
                archives: Vec::from_iter(archive_ids_after),
            },
        }
    }

    fn call_ledger_icrc1_transfer(
        &self,
        from: Principal,
        arg: &TransferArg,
    ) -> Result<Nat, TransferError> {
        Decode!(
            &self.setup.env.execute_ingress_as(
                PrincipalId(from),
                self.ledger_canister_id(),
                "icrc1_transfer",
                Encode!(arg)
                .unwrap()
            )
            .expect("failed to transfer funds")
            .bytes(),
            Result<Nat, TransferError>
        )
        .expect("failed to decode transfer response")
    }

    fn call_ledger_archives(&self) -> Vec<ArchiveInfo> {
        Decode!(
            &self
                .setup
                .env
                .query(self.ledger_canister_id(), "archives", Encode!().unwrap())
                .expect("failed to query archives")
                .bytes(),
            Vec<ArchiveInfo>
        )
        .expect("failed to decode archives response")
    }

    pub fn assert_index_has_correct_ledger_id(self) -> Self {
        assert_eq!(
            self.call_index_ledger_id(),
            self.canister_ids.ledger.unwrap()
        );
        self
    }

    pub fn assert_ledger_has_cycles(self, expected: u128) -> Self {
        assert_eq!(
            self.setup
                .canister_status_of(self.ledger_canister_id())
                .cycles(),
            expected
        );
        self
    }

    pub fn assert_index_has_cycles(self, expected: u128) -> Self {
        assert_eq!(
            self.setup
                .canister_status_of(self.index_canister_id())
                .cycles(),
            expected
        );
        self
    }

    pub fn assert_all_archives_have_cycles(self, expected: u128) -> Self {
        assert!(
            !self.archive_canister_ids().is_empty(),
            "BUG: no archive canisters"
        );
        for archive in self.archive_canister_ids() {
            assert_eq!(self.setup.canister_status_of(archive).cycles(), expected);
        }
        self
    }

    fn call_index_ledger_id(&self) -> Principal {
        Decode!(
            &assert_reply(
                self.setup
                    .env
                    .query(self.index_canister_id(), "ledger_id", Encode!().unwrap())
                    .expect("failed to query get_transactions on the ledger")
            ),
            Principal
        )
        .unwrap()
    }
    pub fn ledger_canister_id(&self) -> CanisterId {
        CanisterId::unchecked_from_principal(PrincipalId::from(self.canister_ids.ledger.unwrap()))
    }

    pub fn index_canister_id(&self) -> CanisterId {
        CanisterId::unchecked_from_principal(PrincipalId::from(self.canister_ids.index.unwrap()))
    }

    pub fn archive_canister_ids(&self) -> Vec<CanisterId> {
        self.canister_ids
            .archives
            .iter()
            .map(|p| CanisterId::unchecked_from_principal(PrincipalId::from(*p)))
            .collect()
    }

    fn all_canister_ids(&self) -> Vec<CanisterId> {
        vec![self.ledger_canister_id(), self.index_canister_id()]
            .into_iter()
            .chain(self.archive_canister_ids())
            .collect()
    }
}

macro_rules! assert_ledger {
    ($name:expr, $ty:ty) => {
        paste::paste! {
            pub fn [<call_ledger_$name:snake >](env: &ic_state_machine_tests::StateMachine, ledger_canister_id: ic_state_machine_tests::CanisterId) -> $ty {
                candid::Decode!(
                    &assert_reply(
                            env
                            .query(ledger_canister_id, $name, candid::Encode!().unwrap())
                            .expect("failed to query on the ledger")
                    ),
                    $ty
                )
                .unwrap()
            }
            impl ManagedCanistersAssert {
                pub fn [<assert_ledger_$name:snake>]<T: Into<$ty>>(self, expected: T) -> Self {
                    assert_eq!([<call_ledger_$name:snake >](&self.setup.env, self.ledger_canister_id()), expected.into(), "BUG: unexpected value for ledger {}", stringify!($name));
                    self
                }
            }
        }
    };
}

assert_ledger!("icrc1_name", String);
assert_ledger!("icrc1_symbol", String);
assert_ledger!("icrc1_decimals", u8);
assert_ledger!("icrc1_total_supply", Nat);
assert_ledger!("icrc1_fee", Nat);
assert_ledger!("icrc1_minting_account", Option<LedgerAccount>);
assert_ledger!("icrc1_metadata", Vec<(String, LedgerMetadataValue)>);
