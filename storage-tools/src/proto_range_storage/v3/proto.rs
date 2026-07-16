pub mod poker {
    pub mod hands {
        pub mod storage {
            pub mod v3 {
                include!(concat!(env!("OUT_DIR"), "/poker.hands.storage.v3.rs"));
            }
        }
    }
}

pub use poker::hands::storage::v3::{
    AbstractActionPathEntry, AbstractActionPathPage, ActionStrategyColumn, ActionType,
    ConcreteActionPathRef, DrillScenarioEntry, DrillScenarioPage, HandEncoding, HandStrategy,
};
