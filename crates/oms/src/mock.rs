//! MockExchange: exchange futures in-memory (one-way, cross) untuk test & mode paper.
//! Mendukung injeksi gagal: algo gagal, close gagal, rate-limit, outcome unknown, MARKET ACK-only.
use crate::exchange::FuturesExchange;
use crate::types::*;
use apex_core::types::Ms;
use rust_decimal::Decimal;
use std::collections::{BTreeMap, VecDeque};
use std::str::FromStr;
use std::sync::Mutex;

#[derive(Clone, Debug)]
struct MockOrder {
    ack: OrderAck,
    kind: OrderKind,
}

#[derive(Clone, Debug)]
struct MockAlgo {
    ack: AlgoAck,
    req: AlgoRequest,
}

#[derive(Default, Debug)]
pub struct MockKnobs {
    pub fail_algo: u32,
    pub fail_reduce_only: u32,
    pub algo_delay_ms: u64,
    pub rate_limit: u32,
    pub unknown_place: u32,
    /// Jumlah panggilan `query_order` yang mengembalikan `Ok(None)` walaupun order sebenarnya ada di bursa.
    pub unknown_invisible_polls: u32,
    /// Bila `true`, `place_order` dengan `unknown_place > 0` gagal sebelum order masuk ke buku bursa
    /// (simulasi paket hilang sebelum menyentuh matching engine).
    pub unknown_drop_before_book: bool,
    pub market_ack_only: bool,
}

#[derive(Default)]
struct Inner {
    now: Ms,
    price: BTreeMap<String, Decimal>,
    orders: BTreeMap<String, MockOrder>,
    algos: BTreeMap<i64, MockAlgo>,
    positions: BTreeMap<String, (Decimal, Decimal)>,
    events: VecDeque<UserEvent>,
    next_id: i64,
    wallet: Decimal,
    leverage: BTreeMap<String, u32>,
    margin: BTreeMap<String, String>,
    one_way: bool,
    knobs: MockKnobs,
    calls: Vec<String>,
}

pub struct MockExchange {
    inner: Mutex<Inner>,
    pub taker: Decimal,
    pub maker: Decimal,
}

impl MockExchange {
    pub fn new(wallet: f64) -> Self {
        MockExchange {
            inner: Mutex::new(Inner {
                wallet: Decimal::from_str(&wallet.to_string()).unwrap_or_default(),
                next_id: 1,
                ..Default::default()
            }),
            taker: Decimal::new(5, 4),
            maker: Decimal::new(2, 4),
        }
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub fn knobs<F: FnOnce(&mut MockKnobs)>(&self, f: F) {
        f(&mut self.lock().knobs)
    }
    pub fn calls(&self) -> Vec<String> {
        self.lock().calls.clone()
    }
    pub fn leverage_of(&self, s: &str) -> Option<u32> {
        self.lock().leverage.get(s).copied()
    }
    pub fn margin_of(&self, s: &str) -> Option<String> {
        self.lock().margin.get(s).cloned()
    }
    pub fn is_one_way(&self) -> bool {
        self.lock().one_way
    }
    pub fn algo_count(&self) -> usize {
        self.lock().algos.len()
    }
    pub fn algos(&self) -> Vec<AlgoRequest> {
        self.lock().algos.values().map(|a| a.req.clone()).collect()
    }
    pub fn position(&self, s: &str) -> Decimal {
        self.lock()
            .positions
            .get(s)
            .map(|p| p.0)
            .unwrap_or_default()
    }
    /// Buat posisi langsung (simulasi posisi yatim untuk test reconcile).
    pub fn inject_position(&self, s: &str, amt: Decimal, entry: Decimal) {
        let mut g = self.lock();
        g.positions.insert(s.into(), (amt, entry));
        g.price.entry(s.into()).or_insert(entry);
    }
    pub fn drain_events(&self) -> Vec<UserEvent> {
        self.lock().events.drain(..).collect()
    }
    pub fn price(&self, s: &str) -> Option<Decimal> {
        self.lock().price.get(s).copied()
    }

    /// Inject a partial limit fill for state-machine tests.
    pub fn partial_fill(&self, cid: &str, qty: Decimal, ts: Ms) {
        let mut g = self.lock();
        g.now = ts;
        let Some(o) = g.orders.get(cid) else { return };
        if qty <= Decimal::ZERO || qty >= o.ack.orig_qty - o.ack.executed_qty {
            return;
        }
        let OrderKind::LimitGtx { price: px } = o.kind else {
            return;
        };
        Self::fill(&mut g, cid, px, qty, self.maker, "LIMIT", false);
    }

    /// Update harga mark/last → isi limit yang tertembus, picu algo.
    pub fn set_price(&self, symbol: &str, px: Decimal, ts: Ms) {
        let mut g = self.lock();
        g.now = ts;
        g.price.insert(symbol.into(), px);
        let fills: Vec<String> = g
            .orders
            .values()
            .filter(|o| o.ack.symbol == symbol && !o.ack.status.is_final())
            .filter(|o| match &o.kind {
                OrderKind::LimitGtx { price } => {
                    (o.ack.side == "BUY" && px < *price) || (o.ack.side == "SELL" && px > *price)
                }
                OrderKind::Market => false,
            })
            .map(|o| o.ack.client_id.clone())
            .collect();
        for cid in fills {
            let (lp, q) = {
                let o = &g.orders[&cid];
                (
                    match o.kind {
                        OrderKind::LimitGtx { price } => price,
                        _ => px,
                    },
                    o.ack.orig_qty - o.ack.executed_qty,
                )
            };
            Self::fill(&mut g, &cid, lp, q, self.maker, "LIMIT", false);
        }
        let trig: Vec<i64> = g
            .algos
            .values()
            .filter(|a| a.req.symbol == symbol)
            .filter(|a| {
                let t = a.req.trigger;
                match (a.req.kind, a.req.side.as_str()) {
                    (AlgoKind::StopMarket, "SELL") | (AlgoKind::TakeProfitMarket, "BUY") => px <= t,
                    (AlgoKind::StopMarket, _) | (AlgoKind::TakeProfitMarket, _) => px >= t,
                }
            })
            .map(|a| a.ack.algo_id)
            .collect();
        for id in trig {
            let Some(a) = g.algos.remove(&id) else {
                continue;
            };
            g.events.push_back(UserEvent::Algo {
                symbol: symbol.into(),
                algo_id: id,
                client_algo_id: a.req.client_algo_id.clone(),
                status: "TRIGGERED".into(),
                ts,
            });
            let pos = g.positions.get(symbol).map(|p| p.0).unwrap_or_default();
            let closes = (a.req.side == "SELL" && pos > Decimal::ZERO)
                || (a.req.side == "BUY" && pos < Decimal::ZERO);
            if closes {
                let q = if a.req.close_position {
                    pos.abs()
                } else {
                    a.req.qty.unwrap_or_default().min(pos.abs())
                };
                let cid = a.req.client_algo_id.clone();
                let id2 = g.next_id;
                g.next_id += 1;
                g.orders.insert(
                    cid.clone(),
                    MockOrder {
                        ack: OrderAck {
                            symbol: symbol.into(),
                            order_id: id2,
                            client_id: cid.clone(),
                            status: OrderStatus::New,
                            side: a.req.side.clone(),
                            orig_qty: q,
                            executed_qty: Decimal::ZERO,
                            avg_price: Decimal::ZERO,
                            reduce_only: true,
                            update_ms: ts,
                        },
                        kind: OrderKind::Market,
                    },
                );
                Self::fill(
                    &mut g,
                    &cid,
                    px,
                    q,
                    self.taker,
                    a.req.kind.binance(),
                    a.req.close_position,
                );
            }
            g.events.push_back(UserEvent::Algo {
                symbol: symbol.into(),
                algo_id: id,
                client_algo_id: a.req.client_algo_id,
                status: "FINISHED".into(),
                ts,
            });
        }
    }

    fn fill(
        g: &mut Inner,
        cid: &str,
        px: Decimal,
        q: Decimal,
        fee_rate: Decimal,
        ot: &str,
        cp: bool,
    ) {
        let ts = g.now;
        let Some(o) = g.orders.get_mut(cid) else {
            return;
        };
        let sym = o.ack.symbol.clone();
        let signed = if o.ack.side == "BUY" { q } else { -q };
        o.ack.executed_qty += q;
        o.ack.avg_price = px;
        o.ack.status = if o.ack.executed_qty < o.ack.orig_qty {
            OrderStatus::PartiallyFilled
        } else {
            OrderStatus::Filled
        };
        o.ack.update_ms = ts;
        let ro = o.ack.reduce_only;
        let side = o.ack.side.clone();
        let oid = o.ack.order_id;
        let cum = o.ack.executed_qty;
        let status = o.ack.status;
        let (amt, entry) = g.positions.get(&sym).copied().unwrap_or_default();
        let new_amt = amt + signed;
        let mut rp = Decimal::ZERO;
        let new_entry = if amt.is_zero() || (amt > Decimal::ZERO) == (signed > Decimal::ZERO) {
            if new_amt.is_zero() {
                Decimal::ZERO
            } else {
                (entry * amt.abs() + px * q) / new_amt.abs()
            }
        } else {
            let closed = q.min(amt.abs());
            rp = if amt > Decimal::ZERO {
                (px - entry) * closed
            } else {
                (entry - px) * closed
            };
            if new_amt.is_zero() {
                Decimal::ZERO
            } else {
                entry
            }
        };
        let fee = px * q * fee_rate;
        g.wallet += rp - fee;
        if new_amt.is_zero() {
            g.positions.remove(&sym);
        } else {
            g.positions.insert(sym.clone(), (new_amt, new_entry));
        }
        g.events.push_back(UserEvent::Order(OrderUpdate {
            symbol: sym.clone(),
            client_id: cid.into(),
            order_id: oid,
            side,
            orig_type: ot.into(),
            status,
            exec_type: "TRADE".into(),
            last_qty: q,
            cum_qty: cum,
            last_price: px,
            avg_price: px,
            fee,
            realized_pnl: rp,
            reduce_only: ro,
            close_position: cp,
            ts,
        }));
        g.events.push_back(UserEvent::Account {
            positions: vec![(sym, new_amt, new_entry)],
            ts,
        });
    }
}

impl FuturesExchange for MockExchange {
    fn name(&self) -> &'static str {
        "mock"
    }
    async fn server_time(&self) -> Result<Ms, ExError> {
        Ok(self.lock().now)
    }
    async fn set_one_way(&self) -> Result<(), ExError> {
        let mut g = self.lock();
        g.calls.push("set_one_way".into());
        g.one_way = true;
        Ok(())
    }
    async fn set_margin_crossed(&self, symbol: &str) -> Result<(), ExError> {
        let mut g = self.lock();
        g.calls.push(format!("margin {symbol} CROSSED"));
        g.margin.insert(symbol.into(), "CROSSED".into());
        Ok(())
    }
    async fn set_leverage(&self, symbol: &str, leverage: u32) -> Result<u32, ExError> {
        let mut g = self.lock();
        g.calls.push(format!("leverage {symbol} {leverage}"));
        g.leverage.insert(symbol.into(), leverage);
        Ok(leverage)
    }
    async fn place_order(&self, req: &OrderRequest) -> Result<OrderAck, ExError> {
        let mut g = self.lock();
        g.calls.push(format!(
            "order {} {} {:?} {} ro={}",
            req.symbol, req.side, req.kind, req.qty, req.reduce_only
        ));
        if g.knobs.rate_limit > 0 {
            g.knobs.rate_limit -= 1;
            return Err(ExError::RateLimited {
                retry_after_ms: 5,
                banned: false,
            });
        }
        if !valid_client_id(&req.client_id) {
            return Err(ExError::Api {
                code: -1102,
                msg: "bad clientOrderId".into(),
                http: 400,
            });
        }
        if g.orders.contains_key(&req.client_id) {
            return Err(ExError::Api {
                code: -4116,
                msg: "ClientOrderId is duplicated".into(),
                http: 400,
            });
        }
        if req.reduce_only {
            if g.knobs.fail_reduce_only > 0 {
                g.knobs.fail_reduce_only -= 1;
                return Err(ExError::Api {
                    code: -1001,
                    msg: "Internal error".into(),
                    http: 500,
                });
            }
            let pos = g
                .positions
                .get(&req.symbol)
                .map(|p| p.0)
                .unwrap_or_default();
            let ok = (req.side == "SELL" && pos > Decimal::ZERO)
                || (req.side == "BUY" && pos < Decimal::ZERO);
            if !ok {
                return Err(ExError::Api {
                    code: -2022,
                    msg: "ReduceOnly Order is rejected".into(),
                    http: 400,
                });
            }
        }
        let px = *g.price.get(&req.symbol).ok_or(ExError::Api {
            code: -1121,
            msg: "no price".into(),
            http: 400,
        })?;
        if let OrderKind::LimitGtx { price } = &req.kind {
            let crosses =
                (req.side == "BUY" && *price >= px) || (req.side == "SELL" && *price <= px);
            if crosses {
                return Err(ExError::Api {
                    code: -5022,
                    msg: "Post Only order will be rejected".into(),
                    http: 400,
                });
            }
        }
        if g.knobs.unknown_place > 0 && g.knobs.unknown_drop_before_book {
            g.knobs.unknown_place -= 1;
            return Err(ExError::Unknown(
                "simulated timeout before matching engine".into(),
            ));
        }
        let id = g.next_id;
        g.next_id += 1;
        let ts = g.now;
        let ack = OrderAck {
            symbol: req.symbol.clone(),
            order_id: id,
            client_id: req.client_id.clone(),
            status: OrderStatus::New,
            side: req.side.clone(),
            orig_qty: req.qty,
            executed_qty: Decimal::ZERO,
            avg_price: Decimal::ZERO,
            reduce_only: req.reduce_only,
            update_ms: ts,
        };
        g.orders.insert(
            req.client_id.clone(),
            MockOrder {
                ack: ack.clone(),
                kind: req.kind.clone(),
            },
        );
        let mut out = ack;
        if req.kind == OrderKind::Market {
            let q = if req.reduce_only {
                req.qty.min(
                    g.positions
                        .get(&req.symbol)
                        .map(|p| p.0.abs())
                        .unwrap_or_default(),
                )
            } else {
                req.qty
            };
            Self::fill(&mut g, &req.client_id, px, q, self.taker, "MARKET", false);
            if !g.knobs.market_ack_only {
                out = g.orders[&req.client_id].ack.clone();
            }
        }
        if g.knobs.unknown_place > 0 {
            g.knobs.unknown_place -= 1;
            return Err(ExError::Unknown("simulated timeout".into()));
        }
        Ok(out)
    }
    async fn query_order(
        &self,
        _symbol: &str,
        client_id: &str,
    ) -> Result<Option<OrderAck>, ExError> {
        let mut g = self.lock();
        g.calls.push(format!("query_order {client_id}"));
        if g.knobs.unknown_invisible_polls > 0 {
            g.knobs.unknown_invisible_polls -= 1;
            return Ok(None);
        }
        Ok(g.orders.get(client_id).map(|o| o.ack.clone()))
    }
    async fn cancel_order(&self, _symbol: &str, client_id: &str) -> Result<(), ExError> {
        let mut g = self.lock();
        g.calls.push(format!("cancel {client_id}"));
        let ts = g.now;
        match g.orders.get_mut(client_id) {
            Some(o) if !o.ack.status.is_final() => {
                o.ack.status = OrderStatus::Canceled;
                let ev = UserEvent::Order(OrderUpdate {
                    symbol: o.ack.symbol.clone(),
                    client_id: client_id.into(),
                    order_id: o.ack.order_id,
                    side: o.ack.side.clone(),
                    orig_type: "LIMIT".into(),
                    status: OrderStatus::Canceled,
                    exec_type: "CANCELED".into(),
                    last_qty: Decimal::ZERO,
                    cum_qty: o.ack.executed_qty,
                    last_price: Decimal::ZERO,
                    avg_price: o.ack.avg_price,
                    fee: Decimal::ZERO,
                    realized_pnl: Decimal::ZERO,
                    reduce_only: o.ack.reduce_only,
                    close_position: false,
                    ts,
                });
                g.events.push_back(ev);
                Ok(())
            }
            _ => Err(ExError::Api {
                code: -2011,
                msg: "Unknown order sent.".into(),
                http: 400,
            }),
        }
    }
    async fn open_orders(&self) -> Result<Vec<OrderAck>, ExError> {
        Ok(self
            .lock()
            .orders
            .values()
            .filter(|o| !o.ack.status.is_final())
            .map(|o| o.ack.clone())
            .collect())
    }
    async fn place_algo(&self, req: &AlgoRequest) -> Result<AlgoAck, ExError> {
        let delay = {
            let mut g = self.lock();
            g.calls.push(format!(
                "algo {} {} {} {} cp={}",
                req.symbol,
                req.side,
                req.kind.binance(),
                req.trigger,
                req.close_position
            ));
            g.knobs.algo_delay_ms
        };
        if delay > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
        }
        let mut g = self.lock();
        if g.knobs.rate_limit > 0 {
            g.knobs.rate_limit -= 1;
            return Err(ExError::RateLimited {
                retry_after_ms: 5,
                banned: false,
            });
        }
        if g.knobs.fail_algo > 0 {
            g.knobs.fail_algo -= 1;
            return Err(ExError::Api {
                code: -1008,
                msg: "Server is currently overloaded".into(),
                http: 503,
            });
        }
        if !valid_client_id(&req.client_algo_id) {
            return Err(ExError::Api {
                code: -1102,
                msg: "bad clientAlgoId".into(),
                http: 400,
            });
        }
        if let Some(px) = g.price.get(&req.symbol).copied() {
            let t = req.trigger;
            let immediate = match (req.kind, req.side.as_str()) {
                (AlgoKind::StopMarket, "SELL") | (AlgoKind::TakeProfitMarket, "BUY") => px <= t,
                _ => px >= t,
            };
            if immediate {
                return Err(ExError::Api {
                    code: -2021,
                    msg: "Order would immediately trigger.".into(),
                    http: 400,
                });
            }
        }
        let id = 1_000_000 + g.next_id;
        g.next_id += 1;
        let ack = AlgoAck {
            algo_id: id,
            client_algo_id: req.client_algo_id.clone(),
            symbol: req.symbol.clone(),
            side: req.side.clone(),
            order_type: req.kind.binance().into(),
            status: "NEW".into(),
            trigger: req.trigger,
        };
        g.algos.insert(
            id,
            MockAlgo {
                ack: ack.clone(),
                req: req.clone(),
            },
        );
        Ok(ack)
    }
    async fn cancel_algo(&self, _symbol: &str, algo_id: i64) -> Result<(), ExError> {
        let mut g = self.lock();
        g.calls.push(format!("cancel_algo {algo_id}"));
        g.algos.remove(&algo_id).map(|_| ()).ok_or(ExError::Api {
            code: -2011,
            msg: "Unknown order sent.".into(),
            http: 400,
        })
    }
    async fn open_algo_orders(&self) -> Result<Vec<AlgoAck>, ExError> {
        Ok(self.lock().algos.values().map(|a| a.ack.clone()).collect())
    }
    async fn positions(&self) -> Result<Vec<PositionInfo>, ExError> {
        let g = self.lock();
        Ok(g.positions
            .iter()
            .map(|(s, (a, e))| PositionInfo {
                symbol: s.clone(),
                amt: *a,
                entry_price: *e,
                mark_price: g.price.get(s).copied().unwrap_or(*e),
                margin_type: g
                    .margin
                    .get(s)
                    .cloned()
                    .unwrap_or_else(|| "CROSSED".into())
                    .to_lowercase(),
                leverage: g.leverage.get(s).copied().unwrap_or(20),
            })
            .collect())
    }
    async fn wallet_balance(&self) -> Result<f64, ExError> {
        Ok(apex_core::num::to_f64(self.lock().wallet))
    }
}
