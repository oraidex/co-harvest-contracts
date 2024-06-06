use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{Addr, Decimal, Uint128};
use cw20::Cw20ReceiveMsg;
use oraiswap::asset::AssetInfo;

use crate::state::{Bid, BidPool, BiddingInfo, Config, DistributionInfo};

#[cw_serde]
pub struct InstantiateMsg {
    pub owner: Addr,
    pub underlying_token: AssetInfo,
    pub distribution_token: AssetInfo,
    pub max_slot: u8,
    pub premium_rate_per_slot: Decimal,
    pub min_deposit_amount: Uint128,
    pub treasury: Addr,
    pub bidding_duration: u64,
}

#[cw_serde]
pub enum ExecuteMsg {
    Receive(Cw20ReceiveMsg),
    UpdateConfig {
        owner: Option<Addr>,
        underlying_token: Option<AssetInfo>,
        distribution_token: Option<AssetInfo>,
        max_slot: Option<u8>,
        premium_rate_per_slot: Option<Decimal>,
        min_deposit_amount: Option<Uint128>,
        treasury: Option<Addr>,
        bidding_duration: Option<u64>,
    },
    CreateNewRound {
        start_time: u64,
        end_time: u64,
        total_distribution: Uint128,
    },
    FinalizeBiddingRoundResult {
        round: u64,
        exchange_rate: Decimal,
    },
    Distribute {
        round: u64,
    },
    SubmitBid {
        round: u64,
        premium_slot: u8,
    },
    CreateNewRoundFromTreasury {},
    UpdateRound {
        idx: u64,
        start_time: Option<u64>,
        end_time: Option<u64>,
        total_distribution: Option<Uint128>,
    },
}

#[cw_serde]
pub enum Cw20HookMsg {
    SubmitBid { round: u64, premium_slot: u8 },
    CreateNewRoundFromTreasury {},
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(Config)]
    Config {},
    #[returns(Bid)]
    Bid { idx: u64 },
    #[returns(BiddingInfoResponse)]
    BiddingInfo { round: u64 },
    #[returns(u64)]
    LastRoundId {},
    #[returns(BidPool)]
    BidPool { round: u64, slot: u8 },
    #[returns(Vec<BidPool>)]
    AllBidPoolInRound { round: u64 },
    #[returns(Vec<Bid>)]
    AllBidInRound {
        round: u64,
        start_after: Option<u64>,
        limit: Option<u64>,
        order_by: Option<i32>,
    },
    #[returns(Vec<u64>)]
    BidsIdxByUser { round: u64, user: Addr },
    #[returns(Vec<Bid>)]
    BidsByUser { round: u64, user: Addr },
    #[returns(EstimateAmountReceiveOfBidResponse)]
    EstimateAmountReceiveOfBid {
        round: u64,
        idx: u64,
        exchange_rate: Decimal,
    },
    #[returns(EstimateAmountReceiveOfBidResponse)]
    EstimateAmountReceive {
        round: u64,
        slot: u8,
        bid_amount: Uint128,
        exchange_rate: Decimal,
    },
    #[returns(u64)]
    NumbersBidInRound { round: u64 },
}

#[cw_serde]
pub struct BiddingInfoResponse {
    pub bid_info: BiddingInfo,
    pub distribution_info: DistributionInfo,
}

#[cw_serde]
pub struct EstimateAmountReceiveOfBidResponse {
    pub receive: Uint128,
    pub residue_bid: Uint128,
}

#[cw_serde]
pub struct MigrateMsg {
    pub owner: Addr,
    pub underlying_token: AssetInfo,
    pub distribution_token: AssetInfo,
    pub max_slot: u8,
    pub premium_rate_per_slot: Decimal,
    pub min_deposit_amount: Uint128,
    pub treasury: Addr,
    pub bidding_duration: u64,
}
