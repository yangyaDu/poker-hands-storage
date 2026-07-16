pub mod zenithstrat {
    pub mod gto {
        pub mod v2 {
            include!(concat!(env!("OUT_DIR"), "/zenithstrat.gto.v2.rs"));
        }
    }
}

pub use zenithstrat::gto::v2::{ActionType, CompactActionColumn, CompactLineMatrix, HandEncoding};
