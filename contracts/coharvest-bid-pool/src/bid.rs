use cosmwasm_std::{
    to_json_binary, Addr, BankMsg, Coin, CosmosMsg, Decimal, DepsMut, Env, MessageInfo, Response,
    StdError, StdResult, Uint128, WasmMsg,
};
use cw20::Cw20ExecuteMsg;
use oraiswap::asset::{Asset, AssetInfo};

use crate::{
    error::ContractError,
    helper::into_cosmos_msg,
    state::{
        pop_bid_idx, read_all_bids_in_round, read_or_create_bid_pool, store_bid, Bid, BidPool,
        BiddingInfo, DistributionInfo, BID, BIDDING_INFO, BID_POOL, CONFIG, DISTRIBUTION_INFO,
        LAST_ROUND_ID,
    },
};

// only owner can call this function
pub fn execute_create_new_round(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    start_time: u64,
    end_time: u64,
    total_distribution: Uint128,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if config.owner != info.sender {
        return Err(ContractError::Unauthorized {});
    }

    // create new bidding round info
    let response = process_create_new_round(deps, env, start_time, end_time, total_distribution)?;

    Ok(response.add_attribute("created_by", "owner"))
}

pub fn execute_create_new_round_from_treasury(
    deps: DepsMut,
    env: Env,
    sender: Addr,
    funds: Asset,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;

    // check the distribute token in the bidding is valid
    assert_token_match_funds(config.distribution_token, funds.info)?;

    // check sender is treasury contract
    if sender != config.treasury {
        return Err(ContractError::Unauthorized {});
    }

    // we are only current new round if last round has ended or still running
    let last_round_id = LAST_ROUND_ID.load(deps.storage)?;

    // if not exist round, new round will create at current time

    let last_round = if last_round_id != 0 {
        BIDDING_INFO.load(deps.storage, last_round_id)?
    } else {
        BiddingInfo {
            round: 0,
            start_time: env.block.time.seconds(),
            end_time: env.block.time.seconds(),
            total_bid_amount: Uint128::zero(),
            total_bid_matched: Uint128::zero(),
        }
    };

    if last_round.start_time > env.block.time.seconds() {
        return Err(ContractError::Std(StdError::generic_err(
            "A new round cannot be created until the last round has started",
        )));
    }

    // startTime = max(current time, end time of last round + 1)
    let start_time = env.block.time.seconds().max(last_round.end_time + 1);
    let end_time = start_time + config.bidding_duration;
    let total_distribution = funds.amount;

    let response = process_create_new_round(deps, env, start_time, end_time, total_distribution)?;

    Ok(response.add_attribute("created_by", "treasury"))
}

fn process_create_new_round(
    deps: DepsMut,
    env: Env,
    start_time: u64,
    end_time: u64,
    total_distribution: Uint128,
) -> Result<Response, ContractError> {
    // create new bidding round info
    let mut last_round = LAST_ROUND_ID.load(deps.storage)?;
    last_round += 1;

    let bidding_info = BiddingInfo {
        round: last_round,
        start_time,
        end_time,
        total_bid_amount: Uint128::zero(),
        total_bid_matched: Uint128::zero(),
    };

    let distribution_info = DistributionInfo {
        total_distribution,
        exchange_rate: Decimal::zero(),
        is_released: false,
        actual_distributed: Uint128::zero(),
        num_bids_distributed: 0,
    };

    if !bidding_info.is_valid_duration(&env) {
        return Err(ContractError::InvalidBiddingTimeRange {});
    }

    // store
    LAST_ROUND_ID.save(deps.storage, &last_round)?;
    BIDDING_INFO.save(deps.storage, last_round, &bidding_info)?;
    DISTRIBUTION_INFO.save(deps.storage, last_round, &distribution_info)?;

    Ok(Response::new().add_attributes(vec![
        ("action", "create_new_bidding_round"),
        ("round", &last_round.to_string()),
        ("start_time", &start_time.to_string()),
        ("end_time", &end_time.to_string()),
        ("reward", &total_distribution.to_string()),
    ]))
}

pub fn execute_update_round(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    idx: u64,
    start_time: Option<u64>,
    end_time: Option<u64>,
    total_distribution: Option<Uint128>,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;

    // check sender is treasury contract
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized {});
    }

    let mut bidding_info = BIDDING_INFO.load(deps.storage, idx)?;
    let mut distribution = DISTRIBUTION_INFO.load(deps.storage, idx)?;

    // cannot update if round is ended
    if bidding_info.finished(&env) {
        return Err(ContractError::RoundEnded {});
    }

    if let Some(total_distribution) = total_distribution {
        distribution.total_distribution = total_distribution;
    }

    if let Some(end_time) = end_time {
        // end time must be gte current time
        if end_time < env.block.time.seconds() {
            return Err(ContractError::InvalidBiddingTimeRange {});
        }

        bidding_info.end_time = end_time;
    }

    if let Some(start_time) = start_time {
        // cannot update if round is staring
        if bidding_info.opening(&env) {
            return Err(ContractError::InvalidBiddingTimeRange {});
        }
        bidding_info.start_time = start_time;
    }

    if !bidding_info.is_valid_duration(&env) {
        return Err(ContractError::InvalidBiddingTimeRange {});
    }

    BIDDING_INFO.save(deps.storage, idx, &bidding_info)?;
    DISTRIBUTION_INFO.save(deps.storage, idx, &distribution)?;

    Ok(Response::new().add_attributes(vec![("action", "update_round")]))
}

//  Underlying asset is submitted to create a bid record
pub fn execute_submit_bid(
    deps: DepsMut,
    env: Env,
    round: u64,
    premium_slot: u8,
    bidder: String,
    funds: Asset,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let amount = funds.amount;
    // check the token participating in the bidding is valid
    assert_token_match_funds(config.underlying_token, funds.info)?;
    if config.min_deposit_amount > amount {
        return Err(ContractError::Std(StdError::generic_err(format!(
            "Minimum deposit is {}, got {}",
            config.min_deposit_amount, amount
        ))));
    }

    if premium_slot < 1 || premium_slot > config.max_slot {
        return Err(ContractError::Std(StdError::generic_err(format!(
            "premium slot must be within the range 1 and {}, reaching {}",
            config.max_slot, premium_slot
        ))));
    }

    // get bid pool info
    let mut bidding_info: BiddingInfo = BIDDING_INFO.load(deps.storage, round)?;

    if !bidding_info.opening(&env) {
        return Err(ContractError::BidNotOpen {});
    }

    // read or create bid_pool, make sure slot is valid
    let mut bid_pool = read_or_create_bid_pool(deps.storage, round, premium_slot)?;
    bidding_info.total_bid_amount += amount;
    bid_pool.total_bid_amount += amount;

    // create bid object
    let bid_idx = pop_bid_idx(deps.storage)?;
    let bid = Bid {
        idx: bid_idx,
        round,
        timestamp: env.block.time.seconds(),
        premium_slot,
        bidder: deps.api.addr_validate(&bidder)?,
        amount,
        residue_bid: amount,
        amount_received: Uint128::zero(),
        is_distributed: false,
    };

    // store bid info
    BIDDING_INFO.save(deps.storage, round, &bidding_info)?;
    BID_POOL.save(deps.storage, (round, premium_slot), &bid_pool)?;
    store_bid(deps.storage, bid_idx, &bid)?;
    Ok(Response::new().add_attributes(vec![
        ("action", "submit_bid"),
        ("round", &round.to_string()),
        ("bidder", &bidder),
        ("bid_idx", &bid_idx.to_string()),
        ("premium_slot", &premium_slot.to_string()),
        ("amount", &amount.to_string()),
    ]))
}

fn assert_token_match_funds(expected: AssetInfo, funds: AssetInfo) -> Result<(), ContractError> {
    if expected.ne(&funds) {
        return Err(ContractError::InvalidFunds {});
    }
    Ok(())
}

// only admin can call this method
// when the bidding round ends, admin will finalized this bidding, update the exchange rate and calculate the amount allocated to all bid pool.
// total number of matched token will be burn. And if after allocation there are still distributed tokens left, send them back to the owner
pub fn execute_finalize_bidding_round_result(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    round: u64,
    exchange_rate: Decimal,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if config.owner != info.sender {
        return Err(ContractError::Unauthorized {});
    }

    let mut bidding_info = BIDDING_INFO.load(deps.storage, round)?;

    // check that bidding round must have ended
    if !bidding_info.finished(&env) {
        return Err(ContractError::BidNotEnded {});
    }

    let mut distribution_info = DISTRIBUTION_INFO.load(deps.storage, round)?;
    if distribution_info.is_released {
        return Err(ContractError::Std(StdError::generic_err(format!(
            "round {} has been finalized",
            round
        ))));
    }

    // update exchange_rate and mark this round as finalized
    distribution_info.exchange_rate = exchange_rate;
    distribution_info.is_released = true;
    let mut bid_pools = bidding_info.read_all_bid_pool(deps.storage)?;

    // calculate the amount allocated to all bid pool
    let mut distribution_amount = distribution_info.total_distribution;
    let total_matched =
        process_calc_distribution_amount(&mut bid_pools, &mut distribution_amount, exchange_rate)?;

    distribution_info.actual_distributed =
        distribution_info.total_distribution - distribution_amount;
    bidding_info.total_bid_matched = total_matched;

    for bid_pool in bid_pools {
        BID_POOL.save(deps.storage, (round, bid_pool.slot), &bid_pool)?;
    }

    DISTRIBUTION_INFO.save(deps.storage, round, &distribution_info)?;
    BIDDING_INFO.save(deps.storage, round, &bidding_info)?;

    let mut msgs: Vec<CosmosMsg> = vec![];

    // burn total_matched
    match config.underlying_token {
        AssetInfo::NativeToken { denom } => msgs.push(CosmosMsg::Bank(BankMsg::Burn {
            amount: vec![Coin {
                denom,
                amount: total_matched,
            }],
        })),
        AssetInfo::Token { contract_addr } => msgs.push(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: contract_addr.to_string(),
            msg: to_json_binary(&Cw20ExecuteMsg::Burn {
                amount: total_matched,
            })?,
            funds: vec![],
        })),
    };

    // transfer remaining to owner
    if !distribution_amount.is_zero() {
        match config.distribution_token {
            AssetInfo::NativeToken { denom } => msgs.push(CosmosMsg::Bank(BankMsg::Send {
                to_address: config.owner.to_string(),
                amount: vec![Coin {
                    denom,
                    amount: distribution_amount,
                }],
            })),
            AssetInfo::Token { contract_addr } => msgs.push(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: contract_addr.to_string(),
                msg: to_json_binary(&Cw20ExecuteMsg::Transfer {
                    recipient: config.owner.to_string(),
                    amount: distribution_amount,
                })?,
                funds: vec![],
            })),
        };
    }

    Ok(Response::new()
        .add_attributes(vec![
            ("action", "finalize_bidding_round_result"),
            ("round", &round.to_string()),
            ("exchange_rate", &exchange_rate.to_string()),
            ("total_matched", &total_matched.to_string()),
            (
                "actual_distributed",
                &distribution_info.actual_distributed.to_string(),
            ),
        ])
        .add_messages(msgs))
}

// after bidding round finalized, call this function to send the allocated tokens to all bidder, and if the bid still has bid token, transfer back to the bidder
pub fn execute_distribute(deps: DepsMut, round: u64) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let mut distribution_info = DISTRIBUTION_INFO.load(deps.storage, round)?;

    if !distribution_info.is_released {
        return Err(ContractError::BidNotEnded {});
    }

    let mut index_snapshot = vec![Decimal::zero(); config.max_slot as usize + 1];
    let mut receiver_per_token = vec![Decimal::zero(); config.max_slot as usize + 1];

    // query all pool in round
    for slot in 1..=config.max_slot {
        if let Some(bid_pool) = BID_POOL.may_load(deps.storage, (round, slot))? {
            index_snapshot[slot as usize] = bid_pool.index_snapshot;
            receiver_per_token[slot as usize] = bid_pool.received_per_token;
        }
    }

    // load all bid in round
    let bids_idx = read_all_bids_in_round(deps.storage, round, None)?;
    let mut msgs: Vec<CosmosMsg> = vec![];

    for idx in bids_idx {
        // read bid
        let mut bid = BID.load(deps.storage, idx)?;
        if bid.is_distributed {
            continue;
        }

        // calc allocated amount and remaining amount of bid
        let amount_received = bid.amount * receiver_per_token[bid.premium_slot as usize];
        let residue_bid = bid.amount * (Decimal::one() - index_snapshot[bid.premium_slot as usize]);

        if amount_received > Uint128::zero() {
            msgs.push(into_cosmos_msg(
                &config.distribution_token,
                bid.bidder.to_string(),
                amount_received,
            )?);
        }

        if residue_bid > Uint128::zero() {
            msgs.push(into_cosmos_msg(
                &config.underlying_token,
                bid.bidder.to_string(),
                residue_bid,
            )?);
        }

        bid.amount_received = amount_received;
        bid.residue_bid = residue_bid;
        bid.is_distributed = true;
        distribution_info.num_bids_distributed += 1;

        BID.save(deps.storage, idx, &bid)?;
    }

    DISTRIBUTION_INFO.save(deps.storage, round, &distribution_info)?;

    Ok(Response::new()
        .add_attributes(vec![
            ("action", "distribute"),
            (
                "total_bids_distributed",
                &distribution_info.num_bids_distributed.to_string(),
            ),
        ])
        .add_messages(msgs))
}

pub fn process_calc_distribution_amount(
    bid_pools: &mut Vec<BidPool>,
    distribution_amount: &mut Uint128,
    exchange_rate: Decimal,
) -> StdResult<Uint128> {
    let mut total_matched = Uint128::zero();

    for bid_pool in bid_pools {
        if bid_pool.total_bid_amount.is_zero() {
            continue;
        }

        let desired_amount =
            bid_pool.total_bid_amount * exchange_rate * (Decimal::one() + bid_pool.premium_rate);

        let actual_amount = if desired_amount <= *distribution_amount {
            desired_amount
        } else {
            *distribution_amount
        };

        let index_snapshot = Decimal::from_ratio(actual_amount, desired_amount);
        let received_per_token = Decimal::from_ratio(actual_amount, bid_pool.total_bid_amount);

        total_matched += index_snapshot * bid_pool.total_bid_amount;
        *distribution_amount -= actual_amount;
        bid_pool.index_snapshot = index_snapshot;
        bid_pool.received_per_token = received_per_token;

        if distribution_amount.is_zero() {
            break;
        }
    }

    Ok(total_matched)
}
