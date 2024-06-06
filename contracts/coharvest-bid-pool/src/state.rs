use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Decimal, Env, Order, StdError, StdResult, Storage, Uint128};
use cw_storage_plus::{Bound, Item, Map};
use oraiswap::asset::AssetInfo;

pub const CONFIG: Item<Config> = Item::new("config");
// mapping (round, slot) --> BiddingPool
pub const BID_POOL: Map<(u64, u8), BidPool> = Map::new("bid_pool");
// mapping round --> BiddingInfo
pub const BIDDING_INFO: Map<u64, BiddingInfo> = Map::new("bidding_info");
pub const LAST_ROUND_ID: Item<u64> = Item::new("last_round_id");
// mapping (round, address) -> vec bid_idx of user
pub const BIDS_BY_USER: Map<(u64, Addr), Vec<u64>> = Map::new("bids_by_user");
// mapping (round, bid_idx) --> (true - bid_idx is included in this round)
pub const BIDS_BY_ROUND: Map<(u64, u64), bool> = Map::new("bids_by_round");
// mapping id --> Bid
pub const BID: Map<u64, Bid> = Map::new("bid");
pub const BID_IDX: Item<u64> = Item::new("bid_idx");
pub const DISTRIBUTION_INFO: Map<u64, DistributionInfo> = Map::new("distribution_info");

const MAX_LIMIT: u64 = 1000;
const DEFAULT_LIMIT: u64 = 30;

#[cw_serde]
pub struct Config {
    pub owner: Addr,                    // owner address
    pub underlying_token: AssetInfo,    // token used to participate in bidding
    pub distribution_token: AssetInfo,  // tokens are used to reward bidding
    pub max_slot: u8,                   // number of pools in a bidding round
    pub premium_rate_per_slot: Decimal, // Premium rate increase for each slot
    pub min_deposit_amount: Uint128,    // minimum number of tokens when participating in bidding
    pub treasury: Addr,                 // treasury address
    pub bidding_duration: u64,          // how long does a bidding round last?
}

#[cw_serde]
pub struct BiddingInfo {
    pub round: u64,                 // round id
    pub start_time: u64,            // start time of the bidding
    pub end_time: u64,              // end time of the bidding
    pub total_bid_amount: Uint128,  // amount of tokens participating in the bidding
    pub total_bid_matched: Uint128, // the number of tokens matched in the bidding
}

#[cw_serde]
pub struct DistributionInfo {
    pub total_distribution: Uint128, // the maximum amount of reward distributed in the bidding
    pub exchange_rate: Decimal, // conversion ratio between underlying_token and distribution_token
    pub is_released: bool,      // mark whether the bidding has been completed or not
    pub actual_distributed: Uint128, // the actual token allocated in the bidding
    pub num_bids_distributed: u64, // number of winning bids in the bidding
}

#[cw_serde]
pub struct BidPool {
    pub slot: u8,                    // the premium slot
    pub total_bid_amount: Uint128,   // number of tokens deposited into this pool
    pub premium_rate: Decimal,       // % bonus of the pool
    pub index_snapshot: Decimal,     // parameter that represents rate at which bids are consumed
    pub received_per_token: Decimal, //  number of reward tokens received for each token deposited into that pool
}

#[cw_serde]
pub struct Bid {
    pub idx: u64,                 // bid id
    pub round: u64,               // bidding round id
    pub premium_slot: u8,         // the premium slot
    pub timestamp: u64,           // time submit bit
    pub bidder: Addr,             // bidder address
    pub amount: Uint128,          // amount of underlying_token put up in bid
    pub residue_bid: Uint128,     // amount of remaining underlying_token
    pub amount_received: Uint128, // amount of tokens allocated
    pub is_distributed: bool,     // mark whether this bid has been allocated or not
}

pub fn pop_bid_idx(storage: &mut dyn Storage) -> StdResult<u64> {
    let last_idx = BID_IDX.load(storage).unwrap_or(1);
    BID_IDX.save(storage, &(last_idx + 1))?;
    Ok(last_idx)
}

pub fn store_bid(storage: &mut dyn Storage, bid_idx: u64, bid: &Bid) -> StdResult<()> {
    BID.save(storage, bid_idx, &bid)?;
    BIDS_BY_USER.update(
        storage,
        (bid.round, bid.bidder.clone()),
        |idxs| -> StdResult<Vec<u64>> {
            let mut idxs = idxs.unwrap_or_default();
            idxs.push(bid_idx);
            Ok(idxs)
        },
    )?;
    BIDS_BY_ROUND.save(storage, (bid.round, bid_idx), &true)?;

    Ok(())
}

pub fn read_or_create_bid_pool(
    storage: &mut dyn Storage,
    round: u64,
    premium_slot: u8,
) -> StdResult<BidPool> {
    let config = CONFIG.load(storage)?;

    match BID_POOL.load(storage, (round, premium_slot)) {
        Ok(bid_pool) => Ok(bid_pool),
        Err(_) => {
            let bid_pool = BidPool {
                slot: premium_slot,
                premium_rate: config.premium_rate_per_slot
                    * Decimal::from_atomics(Uint128::from(premium_slot as u128), 0)
                        .map_err(|err| StdError::generic_err(err.to_string()))?,
                total_bid_amount: Uint128::zero(),
                index_snapshot: Decimal::zero(),
                received_per_token: Decimal::zero(),
            };
            BID_POOL.save(storage, (round, premium_slot), &bid_pool)?;

            Ok(bid_pool)
        }
    }
}

pub fn read_bids_by_round(
    storage: &dyn Storage,
    round: u64,
    start_after: Option<u64>,
    limit: Option<u64>,
    order_by: Option<i32>,
) -> StdResult<Vec<u64>> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
    let order_by = order_by.map_or(Order::Ascending, |val| match val {
        2 => Order::Descending,
        _ => Order::Ascending,
    });
    let start = calc_range_start(start_after)?.map(Bound::ExclusiveRaw);

    BIDS_BY_ROUND
        .prefix(round)
        .keys(storage, start, None, order_by)
        .take(limit)
        .collect()
}

pub fn read_all_bids_in_round(
    storage: &dyn Storage,
    round: u64,
    order_by: Option<i32>,
) -> StdResult<Vec<u64>> {
    let order_by = order_by.map_or(Order::Ascending, |val| match val {
        2 => Order::Descending,
        _ => Order::Ascending,
    });

    BIDS_BY_ROUND
        .prefix(round)
        .keys(storage, None, None, order_by)
        .collect()
}
pub fn count_number_bids_in_round(storage: &dyn Storage, round: u64) -> u64 {
    BIDS_BY_ROUND
        .prefix(round)
        .range(storage, None, None, Order::Ascending)
        .count() as u64
}

impl BiddingInfo {
    pub fn is_valid_duration(&self, env: &Env) -> bool {
        self.start_time < self.end_time && self.start_time >= env.block.time.seconds()
    }

    pub fn opening(&self, env: &Env) -> bool {
        self.start_time <= env.block.time.seconds() && env.block.time.seconds() <= self.end_time
    }

    pub fn finished(&self, env: &Env) -> bool {
        self.end_time < env.block.time.seconds()
    }

    pub fn read_all_bid_pool(&self, storage: &dyn Storage) -> StdResult<Vec<BidPool>> {
        let config = CONFIG.load(storage)?;

        let bid_pools: Vec<BidPool> = (1..=config.max_slot)
            .map(|slot| {
                Ok(BID_POOL
                    .load(storage, (self.round, slot))
                    .unwrap_or(BidPool {
                        slot,
                        total_bid_amount: Uint128::zero(),
                        premium_rate: config.premium_rate_per_slot
                            * Decimal::from_atomics(Uint128::from(slot as u128), 0)
                                .map_err(|err| StdError::generic_err(err.to_string()))?,
                        index_snapshot: Decimal::zero(),
                        received_per_token: Decimal::zero(),
                    }))
            })
            .collect::<StdResult<Vec<BidPool>>>()?;

        Ok(bid_pools)
    }
}

//  this will set the first key after the provided key, by appending a 1 byte
fn calc_range_start(start_after: Option<u64>) -> StdResult<Option<Vec<u8>>> {
    match start_after {
        Some(start) => {
            let mut v: Vec<u8> = start.to_be_bytes().into();
            v.push(0);
            Ok(Some(v))
        }
        None => Ok(None),
    }
}
