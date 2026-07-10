pub mod zenithstrat {
    pub mod gto {
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/zenithstrat.gto.v1.rs"));
        }
    }
}

pub use zenithstrat::gto::v1::{ActionColumn, ActionType, HandEncoding, LineMatrix};
