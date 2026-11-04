//! Trait exchange futures — diimplementasi oleh [`crate::binance::BinanceRest`] dan [`crate::mock::MockExchange`].
use crate::types::*;
use apex_core::types::Ms;

#[allow(async_fn_in_trait)]
pub trait FuturesExchange {
    fn name(&self) -> &'static str;
    async fn server_time(&self) -> Result<Ms, ExError>;
    /// dualSidePosition=false
    async fn set_one_way(&self) -> Result<(), ExError>;
    async fn set_margin_crossed(&self, symbol: &str) -> Result<(), ExError>;
    async fn set_leverage(&self, symbol: &str, leverage: u32) -> Result<u32, ExError>;
    async fn place_order(&self, req: &OrderRequest) -> Result<OrderAck, ExError>;
    async fn query_order(&self, symbol: &str, client_id: &str) -> Result<Option<OrderAck>, ExError>;
    async fn cancel_order(&self, symbol: &str, client_id: &str) -> Result<(), ExError>;
    async fn open_orders(&self) -> Result<Vec<OrderAck>, ExError>;
    async fn place_algo(&self, req: &AlgoRequest) -> Result<AlgoAck, ExError>;
    async fn cancel_algo(&self, symbol: &str, algo_id: i64) -> Result<(), ExError>;
    async fn open_algo_orders(&self) -> Result<Vec<AlgoAck>, ExError>;
    async fn positions(&self) -> Result<Vec<PositionInfo>, ExError>;
    async fn wallet_balance(&self) -> Result<f64, ExError>;
}

/// Delegasi lewat `Arc` (dipakai bersama user-data stream).
impl<T: FuturesExchange> FuturesExchange for std::sync::Arc<T> {
    fn name(&self) -> &'static str {
        (**self).name()
    }
    async fn server_time(&self) -> Result<Ms, ExError> {
        (**self).server_time().await
    }
    async fn set_one_way(&self) -> Result<(), ExError> {
        (**self).set_one_way().await
    }
    async fn set_margin_crossed(&self, symbol: &str) -> Result<(), ExError> {
        (**self).set_margin_crossed(symbol).await
    }
    async fn set_leverage(&self, symbol: &str, leverage: u32) -> Result<u32, ExError> {
        (**self).set_leverage(symbol, leverage).await
    }
    async fn place_order(&self, req: &OrderRequest) -> Result<OrderAck, ExError> {
        (**self).place_order(req).await
    }
    async fn query_order(&self, symbol: &str, client_id: &str) -> Result<Option<OrderAck>, ExError> {
        (**self).query_order(symbol, client_id).await
    }
    async fn cancel_order(&self, symbol: &str, client_id: &str) -> Result<(), ExError> {
        (**self).cancel_order(symbol, client_id).await
    }
    async fn open_orders(&self) -> Result<Vec<OrderAck>, ExError> {
        (**self).open_orders().await
    }
    async fn place_algo(&self, req: &AlgoRequest) -> Result<AlgoAck, ExError> {
        (**self).place_algo(req).await
    }
    async fn cancel_algo(&self, symbol: &str, algo_id: i64) -> Result<(), ExError> {
        (**self).cancel_algo(symbol, algo_id).await
    }
    async fn open_algo_orders(&self) -> Result<Vec<AlgoAck>, ExError> {
        (**self).open_algo_orders().await
    }
    async fn positions(&self) -> Result<Vec<PositionInfo>, ExError> {
        (**self).positions().await
    }
    async fn wallet_balance(&self) -> Result<f64, ExError> {
        (**self).wallet_balance().await
    }
}
