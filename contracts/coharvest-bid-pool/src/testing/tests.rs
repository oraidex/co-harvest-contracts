use std::str::FromStr;

use cosmwasm_std::{
    attr, from_json,
    testing::{mock_dependencies, mock_env, mock_info},
    to_json_binary, Addr, Api, CosmosMsg, Decimal, DepsMut, Env, MessageInfo, OwnedDeps, Querier,
    Response, StdError, Storage, SubMsg, Uint128, WasmMsg,
};
use cw20::{Cw20ExecuteMsg, Cw20ReceiveMsg};
use oraiswap::asset::AssetInfo;

use crate::{
    bid::process_calc_distribution_amount,
    contract::{execute, instantiate, query},
    error::ContractError,
    msg::{
        BiddingInfoResponse, Cw20HookMsg, EstimateAmountReceiveOfBidResponse, ExecuteMsg,
        InstantiateMsg, QueryMsg,
    },
    state::{Bid, BidPool, BiddingInfo, Config, DistributionInfo},
};

const OWNER: &str = "owner";
const ORAIX_ADDR: &str = "orai1lus0f0rhx8s03gdllx2n6vhkmf0536dv57wfge";
const USDC: &str = "orai15un8msx3n5zf9ahlxmfeqd2kwa5wm0nrpxer304m9nd5q6qq0g6sku5pdd";

pub fn init<S: Storage, A: Api, Q: Querier>(deps: &mut OwnedDeps<S, A, Q>) {
    let msg = InstantiateMsg {
        owner: Addr::unchecked(OWNER),
        underlying_token: AssetInfo::Token {
            contract_addr: Addr::unchecked(ORAIX_ADDR),
        },
        distribution_token: AssetInfo::Token {
            contract_addr: Addr::unchecked(USDC),
        },
        max_slot: 25,
        premium_rate_per_slot: Decimal::from_str("0.01").unwrap(),
        min_deposit_amount: Uint128::from(100_000000u128),
        treasury: Addr::unchecked("treasury"),
        bidding_duration: 86400, //
    };

    let info = mock_info(OWNER, &[]);
    instantiate(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();
}

#[test]
fn proper_initialization() {
    let mut deps = mock_dependencies();
    init(&mut deps);

    // check config storge
    let config: Config =
        from_json(&query(deps.as_ref(), mock_env(), QueryMsg::Config {}).unwrap()).unwrap();

    assert_eq!(
        config,
        Config {
            owner: Addr::unchecked(OWNER),
            underlying_token: AssetInfo::Token {
                contract_addr: Addr::unchecked(ORAIX_ADDR),
            },
            distribution_token: AssetInfo::Token {
                contract_addr: Addr::unchecked(USDC),
            },
            max_slot: 25,
            premium_rate_per_slot: Decimal::from_str("0.01").unwrap(),
            min_deposit_amount: Uint128::from(100_000000u128),
            treasury: Addr::unchecked("treasury"),
            bidding_duration: 86400
        }
    )
}

#[test]
fn test_create_new_round() {
    let mut deps = mock_dependencies();
    init(&mut deps);

    let env = mock_env();
    // create failed, unauthorized
    let msg = ExecuteMsg::CreateNewRound {
        start_time: env.block.time.seconds(),
        end_time: env.block.time.plus_seconds(1000).seconds(),
        total_distribution: Uint128::from(20000_000000u128),
    };
    let err = execute(
        deps.as_mut(),
        env.clone(),
        mock_info("addr0001", &vec![]),
        msg.clone(),
    )
    .unwrap_err();
    assert_eq!(err, ContractError::Unauthorized {});

    // create new round success
    let res = execute(deps.as_mut(), env.clone(), mock_info(OWNER, &vec![]), msg).unwrap();
    assert_eq!(
        res.attributes,
        vec![
            attr("action", "create_new_bidding_round"),
            attr("round", "1"),
            attr("start_time", env.block.time.seconds().to_string()),
            attr(
                "end_time",
                env.block.time.plus_seconds(1000).seconds().to_string()
            ),
            attr("reward", Uint128::from(20000_000000u128)),
            attr("created_by", "owner"),
        ]
    );
    // read bidding info & distribution info
    let bidding_info: BiddingInfoResponse = from_json(
        &query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::BiddingInfo { round: 1 },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        bidding_info,
        BiddingInfoResponse {
            bid_info: BiddingInfo {
                round: 1,
                start_time: env.block.time.seconds(),
                end_time: env.block.time.plus_seconds(1000).seconds(),
                total_bid_amount: Uint128::zero(),
                total_bid_matched: Uint128::zero()
            },
            distribution_info: DistributionInfo {
                total_distribution: Uint128::from(20000_000000u128),
                exchange_rate: Decimal::zero(),
                is_released: false,
                actual_distributed: Uint128::zero(),
                num_bids_distributed: 0
            }
        }
    );
}

#[test]
fn test_create_new_round_by_treasury() {
    let mut deps = mock_dependencies();
    init(&mut deps);

    let mut env = mock_env();

    // submit failed, invalid distribution token
    let err = do_create_new_round(
        deps.as_mut(),
        env.clone(),
        mock_info("dummy", &vec![]),
        "addr000".to_string(),
        Uint128::one(),
    )
    .unwrap_err();
    assert_eq!(err, ContractError::InvalidFunds {});

    // submit failed, unauthorized
    let err = do_create_new_round(
        deps.as_mut(),
        env.clone(),
        mock_info(USDC, &vec![]),
        "addr000".to_string(),
        Uint128::one(),
    )
    .unwrap_err();
    assert_eq!(err, ContractError::Unauthorized {});

    // submit success
    let res = do_create_new_round(
        deps.as_mut(),
        env.clone(),
        mock_info(USDC, &vec![]),
        "treasury".to_string(),
        Uint128::one(),
    )
    .unwrap();

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "create_new_bidding_round"),
            attr("round", "1"),
            attr(
                "start_time",
                env.block.time.plus_seconds(1).seconds().to_string()
            ),
            attr(
                "end_time",
                env.block.time.plus_seconds(86401).seconds().to_string()
            ),
            attr("reward", Uint128::one()),
            attr("created_by", "treasury"),
        ]
    );

    // can't create new round if last round not started
    env.block.time = env.block.time.minus_hours(1);
    let err = do_create_new_round(
        deps.as_mut(),
        env.clone(),
        mock_info(USDC, &vec![]),
        "treasury".to_string(),
        Uint128::one(),
    )
    .unwrap_err();
    assert_eq!(
        err,
        ContractError::Std(StdError::generic_err(
            "A new round cannot be created until the last round has started"
        ))
    );
}

#[test]
fn test_update_round() {
    let mut deps = mock_dependencies();
    init(&mut deps);

    let mut env = mock_env();

    let msg = ExecuteMsg::CreateNewRound {
        start_time: env.block.time.seconds(),
        end_time: env.block.time.plus_seconds(1000).seconds(),
        total_distribution: Uint128::from(20000_000000u128),
    };

    execute(deps.as_mut(), env.clone(), mock_info(OWNER, &vec![]), msg).unwrap();

    // update round failed, unauthorized
    let msg = ExecuteMsg::UpdateRound {
        idx: 1,
        start_time: Some(env.block.time.seconds()),
        end_time: Some(env.block.time.plus_seconds(1000).seconds()),
        total_distribution: Some(Uint128::from(20000_000000u128)),
    };

    let err = execute(
        deps.as_mut(),
        env.clone(),
        mock_info("addr000", &vec![]),
        msg.clone(),
    )
    .unwrap_err();

    assert_eq!(err, ContractError::Unauthorized {});

    // round started, cannot update start_time
    let err = execute(deps.as_mut(), env.clone(), mock_info(OWNER, &vec![]), msg).unwrap_err();

    assert_eq!(err, ContractError::InvalidBiddingTimeRange {});

    // end_time < current time => error
    env.block.time = env.block.time.plus_seconds(100);
    let msg = ExecuteMsg::UpdateRound {
        idx: 1,
        start_time: None,
        end_time: Some(env.block.time.minus_seconds(10).seconds()),
        total_distribution: Some(Uint128::from(20000_000000u128)),
    };

    let err = execute(
        deps.as_mut(),
        env.clone(),
        mock_info(OWNER, &vec![]),
        msg.clone(),
    )
    .unwrap_err();

    assert_eq!(err, ContractError::InvalidBiddingTimeRange {});

    // end time < start time
    env.block.time = env.block.time.minus_seconds(200);
    let msg = ExecuteMsg::UpdateRound {
        idx: 1,
        start_time: None,
        end_time: Some(env.block.time.plus_seconds(10).seconds()),
        total_distribution: Some(Uint128::from(20000_000000u128)),
    };

    let err = execute(
        deps.as_mut(),
        env.clone(),
        mock_info(OWNER, &vec![]),
        msg.clone(),
    )
    .unwrap_err();

    assert_eq!(err, ContractError::InvalidBiddingTimeRange {});

    // update success

    let msg = ExecuteMsg::UpdateRound {
        idx: 1,
        start_time: None,
        end_time: Some(env.block.time.plus_seconds(1000).seconds()),
        total_distribution: Some(Uint128::from(20000_000000u128)),
    };

    execute(
        deps.as_mut(),
        env.clone(),
        mock_info(OWNER, &vec![]),
        msg.clone(),
    )
    .unwrap();

    // read bidding info & distribution info
    let bidding_info: BiddingInfoResponse = from_json(
        &query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::BiddingInfo { round: 1 },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        bidding_info,
        BiddingInfoResponse {
            bid_info: BiddingInfo {
                round: 1,
                start_time: env.block.time.plus_seconds(100).seconds(),
                end_time: env.block.time.plus_seconds(1000).seconds(),
                total_bid_amount: Uint128::zero(),
                total_bid_matched: Uint128::zero()
            },
            distribution_info: DistributionInfo {
                total_distribution: Uint128::from(20000_000000u128),
                exchange_rate: Decimal::zero(),
                is_released: false,
                actual_distributed: Uint128::zero(),
                num_bids_distributed: 0
            }
        }
    );
}

#[test]
fn test_submit_bids_and_querier() {
    let mut deps = mock_dependencies();
    init(&mut deps);

    let mut env = mock_env();
    // create failed, unauthorized
    let msg = ExecuteMsg::CreateNewRound {
        start_time: env.block.time.seconds(),
        end_time: env.block.time.plus_seconds(1000).seconds(),
        total_distribution: Uint128::from(20000_000000u128),
    };
    let err = execute(
        deps.as_mut(),
        env.clone(),
        mock_info("addr0001", &vec![]),
        msg.clone(),
    )
    .unwrap_err();
    assert_eq!(err, ContractError::Unauthorized {});

    // create new round success
    execute(deps.as_mut(), env.clone(), mock_info(OWNER, &vec![]), msg).unwrap();

    // try submit invalid token
    let err = do_submit_bid(
        deps.as_mut(),
        env.clone(),
        mock_info("dummy", &vec![]),
        "addr000".to_string(),
        Uint128::one(),
        1,
        1,
    )
    .unwrap_err();
    assert_eq!(err, ContractError::InvalidFunds {});

    // try submit to the bidding with amount is less than minimum deposit
    env.block.time = env.block.time.minus_seconds(100);
    let err = do_submit_bid(
        deps.as_mut(),
        env.clone(),
        mock_info(ORAIX_ADDR, &vec![]),
        "addr000".to_string(),
        Uint128::one(),
        1,
        1,
    )
    .unwrap_err();
    assert_eq!(
        err,
        ContractError::Std(StdError::generic_err("Minimum deposit is 100000000, got 1"))
    );

    // try submit to the bidding round that hasn't started yet
    env.block.time = env.block.time.minus_seconds(100);
    let err = do_submit_bid(
        deps.as_mut(),
        env.clone(),
        mock_info(ORAIX_ADDR, &vec![]),
        "addr000".to_string(),
        Uint128::from(100_000000u128),
        1,
        1,
    )
    .unwrap_err();
    assert_eq!(err, ContractError::BidNotOpen {});

    // submit bid success
    env = mock_env();
    let res = do_submit_bid(
        deps.as_mut(),
        env.clone(),
        mock_info(ORAIX_ADDR, &vec![]),
        "addr000".to_string(),
        Uint128::from(100_000000u128),
        1,
        1,
    )
    .unwrap();

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "submit_bid"),
            attr("round", "1"),
            attr("bidder", "addr000"),
            attr("bid_idx", "1"),
            attr("premium_slot", "1"),
            attr("amount", "100000000")
        ]
    );
    // query bid info
    let bid: Bid =
        from_json(&query(deps.as_ref(), mock_env(), QueryMsg::Bid { idx: 1 }).unwrap()).unwrap();
    assert_eq!(
        bid,
        Bid {
            idx: 1,
            round: 1,
            bidder: Addr::unchecked("addr000"),
            timestamp: mock_env().block.time.seconds(),
            amount: Uint128::from(100_000000u128),
            residue_bid: Uint128::from(100_000000u128),
            premium_slot: 1,
            amount_received: Uint128::zero(),
            is_distributed: false
        }
    );

    // try submit other bid with the same slot
    do_submit_bid(
        deps.as_mut(),
        env.clone(),
        mock_info(ORAIX_ADDR, &vec![]),
        "addr000".to_string(),
        Uint128::from(200_000000u128),
        1,
        1,
    )
    .unwrap();

    // try submit other bid from another user
    do_submit_bid(
        deps.as_mut(),
        env.clone(),
        mock_info(ORAIX_ADDR, &vec![]),
        "addr001".to_string(),
        Uint128::from(300_000000u128),
        1,
        2,
    )
    .unwrap();

    // query bid info
    let bid_pool: BidPool = from_json(
        &query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::BidPool { round: 1, slot: 1 },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        bid_pool,
        BidPool {
            total_bid_amount: Uint128::from(300_000000u128),
            premium_rate: Decimal::from_str("0.01").unwrap(),
            index_snapshot: Decimal::zero(),
            received_per_token: Decimal::zero(),
            slot: 1
        }
    );
    // read bidding info & distribution info
    let bidding_info: BiddingInfoResponse = from_json(
        &query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::BiddingInfo { round: 1 },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        bidding_info,
        BiddingInfoResponse {
            bid_info: BiddingInfo {
                round: 1,
                start_time: env.block.time.seconds(),
                end_time: env.block.time.plus_seconds(1000).seconds(),
                total_bid_amount: Uint128::from(600_000000u128),
                total_bid_matched: Uint128::zero()
            },
            distribution_info: DistributionInfo {
                total_distribution: Uint128::from(20000_000000u128),
                exchange_rate: Decimal::zero(),
                is_released: false,
                actual_distributed: Uint128::zero(),
                num_bids_distributed: 0u64,
            }
        }
    );

    // query all bid of bid pools
    let bid_pools = bidding_info
        .bid_info
        .read_all_bid_pool(deps.as_ref().storage)
        .unwrap();
    assert_eq!(
        bid_pools[0],
        BidPool {
            slot: 1,
            total_bid_amount: Uint128::from(300_000000u128),
            premium_rate: Decimal::from_str("0.01").unwrap(),
            index_snapshot: Decimal::zero(),
            received_per_token: Decimal::zero()
        }
    );
    assert_eq!(
        bid_pools[1],
        BidPool {
            slot: 2,
            total_bid_amount: Uint128::from(300_000000u128),
            premium_rate: Decimal::from_str("0.02").unwrap(),
            index_snapshot: Decimal::zero(),
            received_per_token: Decimal::zero()
        }
    );
    for i in 2..bid_pools.len() {
        assert_eq!(
            bid_pools[i],
            BidPool {
                slot: i as u8 + 1,
                total_bid_amount: Uint128::zero(),
                premium_rate: Decimal::from_ratio(i as u128 + 1, 100u128),
                index_snapshot: Decimal::zero(),
                received_per_token: Decimal::zero()
            }
        );
    }

    // query all bid by bid_pool
    let bids: Vec<u64> = from_json(
        &query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::AllBidInRound {
                round: 1,
                start_after: None,
                limit: None,
                order_by: None,
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(bids, vec![1, 2, 3]);

    let bids_by_users: Vec<u64> = from_json(
        &query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::BidsIdxByUser {
                round: 1,
                user: Addr::unchecked("addr000"),
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(bids_by_users, vec![1, 2]);
}

#[test]
fn test_full_amount_to_be_distributed() {
    let mut bid_pools: Vec<BidPool> = vec![];

    // totalBid = 100000
    for slot in 1..=25 {
        bid_pools.push(BidPool {
            slot,
            total_bid_amount: Uint128::from(4000_000000u128),
            premium_rate: Decimal::from_ratio(slot as u128, 100u128),
            index_snapshot: Decimal::zero(),
            received_per_token: Decimal::zero(),
        });
    }

    // totalBid = 25 * 4000 = 100000
    // exchangeRate = 0.01
    // => distributionAmount need to fill completely: 4000*1.01*0.01 + 4000*1.02*0.01 + ... + 4000*1.25*0.01 = 4000*0.01*(1.01+1.02+..1.25) = 4000 * 0.01 * 28.25 = 1130
    let mut distribution_amount = Uint128::from(1130_000000u128);
    let exchange_rate = Decimal::from_ratio(1u128, 100u128);

    let total_matched =
        process_calc_distribution_amount(&mut bid_pools, &mut distribution_amount, exchange_rate)
            .unwrap();

    assert_eq!(total_matched, Uint128::from(100000_000000u128));
    assert!(distribution_amount.is_zero());

    for bid_pool in bid_pools {
        assert_eq!(bid_pool.index_snapshot, Decimal::one());
        assert_eq!(
            (Decimal::one() + bid_pool.premium_rate) * exchange_rate,
            bid_pool.received_per_token
        );
    }
}

#[test]
fn test_partial_amount_to_be_distributed() {
    let mut bid_pools: Vec<BidPool> = vec![];

    // totalBid = 96000
    for slot in 1..=24 {
        bid_pools.push(BidPool {
            slot,
            total_bid_amount: Uint128::from(4000_000000u128),
            premium_rate: Decimal::from_ratio(slot as u128, 100u128),
            index_snapshot: Decimal::zero(),
            received_per_token: Decimal::zero(),
        });
    }

    // totalBid = 24 * 4000 = 96000
    // exchangeRate = 0.01
    let mut distribution_amount = Uint128::from(1130_000000u128);
    let exchange_rate = Decimal::from_ratio(1u128, 100u128);

    let total_matched =
        process_calc_distribution_amount(&mut bid_pools, &mut distribution_amount, exchange_rate)
            .unwrap();

    assert_eq!(total_matched, Uint128::from(96000_000000u128));

    assert_eq!(distribution_amount, Uint128::from(50_000000u128));

    for bid_pool in bid_pools {
        assert_eq!(bid_pool.index_snapshot, Decimal::one());
        assert_eq!(
            (Decimal::one() + bid_pool.premium_rate) * exchange_rate,
            bid_pool.received_per_token
        );
    }
}

#[test]
fn test_one_bid_pool_is_partially_matched() {
    let mut bid_pools: Vec<BidPool> = vec![];

    // Assume have 2 bid_pool at slot 10 & 20
    bid_pools.push(BidPool {
        slot: 10,
        total_bid_amount: Uint128::from(1000_000000u128),
        premium_rate: Decimal::from_ratio(10u128, 100u128),
        index_snapshot: Decimal::zero(),
        received_per_token: Decimal::zero(),
    });
    bid_pools.push(BidPool {
        slot: 20,
        total_bid_amount: Uint128::from(1000_000000u128),
        premium_rate: Decimal::from_ratio(20u128, 100u128),
        index_snapshot: Decimal::zero(),
        received_per_token: Decimal::zero(),
    });

    let mut distribution_amount = Uint128::from(20_000000u128);
    let exchange_rate = Decimal::from_ratio(1u128, 100u128);

    // pool at slot 10:  fulfilled (1000*1.1*0.01 = 11) => remaining 9
    // the remaining are distributed to pool at slot 20
    // totalMatch = 1000 + 9 / 12 * 1000 = 1750
    let total_matched =
        process_calc_distribution_amount(&mut bid_pools, &mut distribution_amount, exchange_rate)
            .unwrap();
    assert_eq!(total_matched, Uint128::from(1750_000000u128));
    assert_eq!(distribution_amount, Uint128::zero());

    assert_eq!(
        bid_pools[0],
        BidPool {
            slot: 10,
            total_bid_amount: Uint128::from(1000_000000u128),
            premium_rate: Decimal::from_ratio(10u128, 100u128),
            index_snapshot: Decimal::one(),
            received_per_token: Decimal::from_ratio(11u128, 1000u128),
        }
    );
    assert_eq!(
        bid_pools[1],
        BidPool {
            slot: 20,
            total_bid_amount: Uint128::from(1000_000000u128),
            premium_rate: Decimal::from_ratio(20u128, 100u128),
            index_snapshot: Decimal::from_ratio(3u128, 4u128),
            received_per_token: Decimal::from_ratio(9u128, 1000u128),
        }
    )
}

#[test]
fn test_all_bid_matched_but_distribution_amount_remains() {
    let mut bid_pools: Vec<BidPool> = vec![];

    // totalBid = 96000
    for slot in 1..=25 {
        bid_pools.push(BidPool {
            slot,
            total_bid_amount: Uint128::from(4000_000000u128),
            premium_rate: Decimal::from_ratio(slot as u128, 100u128),
            index_snapshot: Decimal::zero(),
            received_per_token: Decimal::zero(),
        });
    }

    // totalBid = 25 * 4000 = 100000
    // exchangeRate = 0.01
    // => actual distribute = 1130
    let mut distribution_amount = Uint128::from(1200_000000u128);
    let exchange_rate = Decimal::from_ratio(1u128, 100u128);

    let total_matched =
        process_calc_distribution_amount(&mut bid_pools, &mut distribution_amount, exchange_rate)
            .unwrap();
    assert_eq!(total_matched, Uint128::from(100000_000000u128));
    assert_eq!(distribution_amount, Uint128::from(70_000000u128));
}

#[test]
fn test_finalize_bidding_round_result() {
    let mut deps = mock_dependencies();
    init(&mut deps);

    // fulfilled
    let mut env = mock_env();
    let msg = ExecuteMsg::CreateNewRound {
        start_time: env.block.time.seconds(),
        end_time: env.block.time.plus_seconds(1000).seconds(),
        total_distribution: Uint128::from(1080_000000u128),
    };
    execute(deps.as_mut(), env.clone(), mock_info(OWNER, &vec![]), msg).unwrap();

    for i in 1..=25 {
        do_submit_bid(
            deps.as_mut(),
            env.clone(),
            mock_info(ORAIX_ADDR, &vec![]),
            "addr000".to_string(),
            Uint128::from(4000_000000u128),
            1,
            i,
        )
        .unwrap();
    }

    // finalize error, unauthorized
    let msg = ExecuteMsg::FinalizeBiddingRoundResult {
        round: 1,
        exchange_rate: Decimal::from_ratio(1u128, 100u128),
    };
    let err = execute(
        deps.as_mut(),
        env.clone(),
        mock_info("addr000", &vec![]),
        msg.clone(),
    )
    .unwrap_err();
    assert_eq!(err, ContractError::Unauthorized {});

    // finalize error, this round has not ended
    let err = execute(
        deps.as_mut(),
        env.clone(),
        mock_info(OWNER, &vec![]),
        msg.clone(),
    )
    .unwrap_err();
    assert_eq!(err, ContractError::BidNotEnded {});

    // finalize success
    env.block.time = env.block.time.plus_seconds(1001);
    let res = execute(
        deps.as_mut(),
        env.clone(),
        mock_info(OWNER, &vec![]),
        msg.clone(),
    )
    .unwrap();

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "finalize_bidding_round_result"),
            attr("round", "1"),
            attr("exchange_rate", "0.01"),
            attr("total_matched", "96000000000"),
            attr("actual_distributed", "1080000000"),
        ]
    );

    assert_eq!(
        res.messages,
        vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: ORAIX_ADDR.to_string(),
            msg: to_json_binary(&Cw20ExecuteMsg::Burn {
                amount: Uint128::from(96000_000000u128)
            })
            .unwrap(),
            funds: vec![]
        }))]
    );

    // case 2: all_bid_matched_but_distribution_amount_remains
    let msg = ExecuteMsg::CreateNewRound {
        start_time: env.block.time.seconds(),
        end_time: env.block.time.plus_seconds(1000).seconds(),
        total_distribution: Uint128::from(1200_000000u128),
    };
    execute(deps.as_mut(), env.clone(), mock_info(OWNER, &vec![]), msg).unwrap();

    for i in 1..=25 {
        do_submit_bid(
            deps.as_mut(),
            env.clone(),
            mock_info(ORAIX_ADDR, &vec![]),
            "addr000".to_string(),
            Uint128::from(4000_000000u128),
            2,
            i,
        )
        .unwrap();
    }
    let msg = ExecuteMsg::FinalizeBiddingRoundResult {
        round: 2,
        exchange_rate: Decimal::from_ratio(1u128, 100u128),
    };
    env.block.time = env.block.time.plus_seconds(1001);
    let res = execute(
        deps.as_mut(),
        env.clone(),
        mock_info(OWNER, &vec![]),
        msg.clone(),
    )
    .unwrap();
    assert_eq!(
        res.attributes,
        vec![
            attr("action", "finalize_bidding_round_result"),
            attr("round", "2"),
            attr("exchange_rate", "0.01"),
            attr("total_matched", "100000000000"),
            attr("actual_distributed", "1130000000"),
        ]
    );

    assert_eq!(
        res.messages,
        vec![
            SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: ORAIX_ADDR.to_string(),
                msg: to_json_binary(&Cw20ExecuteMsg::Burn {
                    amount: Uint128::from(100000_000000u128)
                })
                .unwrap(),
                funds: vec![]
            })),
            SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: USDC.to_string(),
                msg: to_json_binary(&Cw20ExecuteMsg::Transfer {
                    recipient: OWNER.to_string(),
                    amount: Uint128::from(70_000000u128)
                })
                .unwrap(),
                funds: vec![],
            }))
        ]
    );
}

#[test]
fn test_distribute() {
    let mut deps = mock_dependencies();
    init(&mut deps);

    // all bid filled
    let mut env = mock_env();
    let msg = ExecuteMsg::CreateNewRound {
        start_time: env.block.time.seconds(),
        end_time: env.block.time.plus_seconds(1000).seconds(),
        total_distribution: Uint128::from(1200_000000u128),
    };
    execute(deps.as_mut(), env.clone(), mock_info(OWNER, &vec![]), msg).unwrap();

    for i in 1..=25 {
        do_submit_bid(
            deps.as_mut(),
            env.clone(),
            mock_info(ORAIX_ADDR, &vec![]),
            "addr000".to_string(),
            Uint128::from(4000_000000u128),
            1,
            i,
        )
        .unwrap();
    }
    let msg = ExecuteMsg::FinalizeBiddingRoundResult {
        round: 1,
        exchange_rate: Decimal::from_ratio(1u128, 100u128),
    };
    env.block.time = env.block.time.plus_seconds(1001);
    execute(
        deps.as_mut(),
        env.clone(),
        mock_info(OWNER, &vec![]),
        msg.clone(),
    )
    .unwrap();

    // query total bid in this round
    let num_bids_in_round: u64 = from_json(
        &query(
            deps.as_ref(),
            env.clone(),
            QueryMsg::NumbersBidInRound { round: 1 },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(num_bids_in_round, 25);

    let msg = ExecuteMsg::Distribute { round: 1 };
    let res = execute(
        deps.as_mut(),
        env.clone(),
        mock_info("addr000", &vec![]),
        msg.clone(),
    )
    .unwrap();

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "distribute"),
            attr("total_bids_distributed", "25"),
        ]
    );

    let msgs: Vec<SubMsg> = (1..=25)
        .map(|i| {
            SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: USDC.to_string(),
                msg: to_json_binary(&Cw20ExecuteMsg::Transfer {
                    recipient: "addr000".to_string(),
                    amount: Uint128::from(4000_000000u128)
                        * Decimal::from_ratio((100 + i) as u128, 100u128)
                        * Decimal::from_ratio(1u128, 100u128),
                })
                .unwrap(),
                funds: vec![],
            }))
        })
        .collect();

    assert_eq!(res.messages, msgs);

    // 23 bid filled, bid 24-th partial fill, 25-th not fill
    let msg = ExecuteMsg::CreateNewRound {
        start_time: env.block.time.seconds(),
        end_time: env.block.time.plus_seconds(1000).seconds(),
        total_distribution: Uint128::from(1055_200000u128),
    };
    execute(deps.as_mut(), env.clone(), mock_info(OWNER, &vec![]), msg).unwrap();

    for i in 1..=25 {
        do_submit_bid(
            deps.as_mut(),
            env.clone(),
            mock_info(ORAIX_ADDR, &vec![]),
            "addr000".to_string(),
            Uint128::from(4000_000000u128),
            2,
            i,
        )
        .unwrap();
    }
    let msg = ExecuteMsg::FinalizeBiddingRoundResult {
        round: 2,
        exchange_rate: Decimal::from_ratio(1u128, 100u128),
    };
    env.block.time = env.block.time.plus_seconds(1001);
    execute(
        deps.as_mut(),
        env.clone(),
        mock_info(OWNER, &vec![]),
        msg.clone(),
    )
    .unwrap();

    // query total bid in this round
    let num_bids_in_round: u64 = from_json(
        &query(
            deps.as_ref(),
            env.clone(),
            QueryMsg::NumbersBidInRound { round: 1 },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(num_bids_in_round, 25);

    let msg = ExecuteMsg::Distribute { round: 2 };
    let res = execute(
        deps.as_mut(),
        env.clone(),
        mock_info("addr000", &vec![]),
        msg.clone(),
    )
    .unwrap();

    assert_eq!(
        res.attributes,
        vec![
            attr("action", "distribute"),
            attr("total_bids_distributed", "25"),
        ]
    );

    let mut msgs: Vec<SubMsg> = (1..=23)
        .map(|i| {
            SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: USDC.to_string(),
                msg: to_json_binary(&Cw20ExecuteMsg::Transfer {
                    recipient: "addr000".to_string(),
                    amount: Uint128::from(4000_000000u128)
                        * Decimal::from_ratio((100 + i) as u128, 100u128)
                        * Decimal::from_ratio(1u128, 100u128),
                })
                .unwrap(),
                funds: vec![],
            }))
        })
        .collect();
    // bid 24-th filled a-half
    msgs.push(SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: USDC.to_string(),
        msg: to_json_binary(&Cw20ExecuteMsg::Transfer {
            recipient: "addr000".to_string(),
            amount: Uint128::from(4000_000000u128)
                * Decimal::from_ratio((100 + 24) as u128, 100u128)
                * Decimal::from_ratio(1u128, 100u128)
                * Decimal::from_ratio(1u128, 2u128),
        })
        .unwrap(),
        funds: vec![],
    })));
    msgs.push(SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: ORAIX_ADDR.to_string(),
        msg: to_json_binary(&Cw20ExecuteMsg::Transfer {
            recipient: "addr000".to_string(),
            amount: Uint128::from(4000_000000u128) * Decimal::from_ratio(1u128, 2u128),
        })
        .unwrap(),
        funds: vec![],
    })));

    // bid 25-th not fill
    msgs.push(SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: ORAIX_ADDR.to_string(),
        msg: to_json_binary(&Cw20ExecuteMsg::Transfer {
            recipient: "addr000".to_string(),
            amount: Uint128::from(4000_000000u128),
        })
        .unwrap(),
        funds: vec![],
    })));

    assert_eq!(res.messages, msgs);
}

#[test]
fn test_estimate_token_received() {
    let mut deps = mock_dependencies();
    init(&mut deps);

    // all bid filled
    let env = mock_env();
    let msg = ExecuteMsg::CreateNewRound {
        start_time: env.block.time.seconds(),
        end_time: env.block.time.plus_seconds(1000).seconds(),
        total_distribution: Uint128::from(1130_000000u128),
    };
    execute(deps.as_mut(), env.clone(), mock_info(OWNER, &vec![]), msg).unwrap();

    for i in 1..=25 {
        do_submit_bid(
            deps.as_mut(),
            env.clone(),
            mock_info(ORAIX_ADDR, &vec![]),
            "addr000".to_string(),
            Uint128::from(4000_000000u128),
            1,
            i,
        )
        .unwrap();
    }

    // All bids will be matched
    let res: EstimateAmountReceiveOfBidResponse = from_json(
        &query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::EstimateAmountReceiveOfBid {
                round: 1,
                idx: 10,
                exchange_rate: Decimal::from_ratio(1u128, 100u128),
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        res,
        EstimateAmountReceiveOfBidResponse {
            receive: Uint128::from(44_000000u128),
            residue_bid: Uint128::zero()
        }
    );

    // because all bids will be matched, so say submit another bid at slot 25 with 4000 tokens ==> all bids at slot 25 will match only half
    let res: EstimateAmountReceiveOfBidResponse = from_json(
        &query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::EstimateAmountReceive {
                round: 1,
                slot: 25,
                bid_amount: Uint128::from(4000_000000u128),
                exchange_rate: Decimal::from_ratio(1u128, 100u128),
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        res,
        EstimateAmountReceiveOfBidResponse {
            receive: Uint128::from(25_000000u128),
            residue_bid: Uint128::from(2000_000000u128),
        }
    );

    // try submit this bid
    do_submit_bid(
        deps.as_mut(),
        env.clone(),
        mock_info(ORAIX_ADDR, &vec![]),
        "addr000".to_string(),
        Uint128::from(4000_000000u128),
        1,
        25,
    )
    .unwrap();
    let res: EstimateAmountReceiveOfBidResponse = from_json(
        &query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::EstimateAmountReceiveOfBid {
                round: 1,
                idx: 26,
                exchange_rate: Decimal::from_ratio(1u128, 100u128),
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        res,
        EstimateAmountReceiveOfBidResponse {
            receive: Uint128::from(25_000000u128),
            residue_bid: Uint128::from(2000_000000u128),
        }
    );
}
pub fn do_submit_bid(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    sender: String,
    amount: Uint128,
    round: u64,
    premium_slot: u8,
) -> Result<Response, ContractError> {
    let msg = Cw20HookMsg::SubmitBid {
        round,
        premium_slot,
    };
    let receive = ExecuteMsg::Receive(Cw20ReceiveMsg {
        sender,
        amount,
        msg: to_json_binary(&msg).unwrap(),
    });

    execute(deps, env, info, receive)
}

pub fn do_create_new_round(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    sender: String,
    amount: Uint128,
) -> Result<Response, ContractError> {
    let msg = Cw20HookMsg::CreateNewRoundFromTreasury {};

    let receive = ExecuteMsg::Receive(Cw20ReceiveMsg {
        sender,
        amount,
        msg: to_json_binary(&msg).unwrap(),
    });

    execute(deps, env, info, receive)
}
