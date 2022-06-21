use cosmwasm_std::{
    coins, entry_point, to_binary, BankMsg, Binary, Coin, Deps, DepsMut, Env, MessageInfo, Order,
    Response, StdError, StdResult, Uint128,
};
use cw_storage_plus::Bound;

use crate::error::ContractError;
use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};
use crate::state::{Game, GameBoard, GameResult, GameState, GAME_MAP};

// settings for pagination
const MAX_LIMIT: u32 = 30;
const DEFAULT_LIMIT: u32 = 10;

// Note, you can use StdResult in some functions where you do not
// make use of the custom errors
#[entry_point]
pub fn instantiate(
    _deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    _msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    Ok(Response::default())
}

// And declare a custom Error variant for the ones where you will want to make use of it
#[entry_point]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::CreateGame { bet } => try_create_game(deps, env, info, bet),
        ExecuteMsg::JoinGame { game_id } => try_join_game(deps, info, game_id),
        ExecuteMsg::WithdrawBet { game_id } => try_withdraw_bets(deps, info, game_id),
        ExecuteMsg::UpdateGame {
            game_id,
            side,
            i,
            j,
        } => try_update_game(deps, info, game_id, i, j, side),
        ExecuteMsg::CancelGame { game_id } => try_cancel_game(deps, info, game_id),
    }
}

fn try_cancel_game(
    deps: DepsMut,
    info: MessageInfo,
    game_id: Uint128,
) -> Result<Response, ContractError> {
    nonpayable(&info)?;
    let mut game = GAME_MAP
        .may_load(deps.storage, game_id.to_string())?
        .unwrap();
    if game.state != GameState::Pending {
        return Err(ContractError::Std(StdError::generic_err(
            "Game is not in Pending",
        )));
    }
    if game.cross != info.sender {
        return Err(ContractError::Unauthorized {});
    }
    game.complete_game();
    GAME_MAP.save(deps.storage, game_id.to_string(), &game)?;
    return Ok(Response::new().add_message(BankMsg::Send {
        to_address: game.cross.to_string(),
        amount: vec![game.bet],
    }));
}

fn try_withdraw_bets(
    deps: DepsMut,
    info: MessageInfo,
    game_id: Uint128,
) -> Result<Response, ContractError> {
    nonpayable(&info)?;
    let mut game = GAME_MAP
        .may_load(deps.storage, game_id.to_string())?
        .unwrap();
    if game.state != GameState::Started {
        return Err(ContractError::Std(StdError::generic_err(
            "Game is Pending or Completed",
        )));
    }
    let validated_game = find_winner_by_board(game.game)?;
    let res = Response::default();
    let game_bet = game.bet.amount.checked_mul(Uint128::from(2u128)).unwrap();
    match validated_game {
        GameResult::Cross => {
            game.complete_game();
            GAME_MAP.save(deps.storage, game_id.to_string(), &game)?;
            let msg = BankMsg::Send {
                to_address: game.cross.to_string(),
                amount: coins(game_bet.u128(), game.bet.denom),
            };
            Ok(res
                .add_message(msg)
                .add_attribute("to", game.cross.to_string())
                .add_attribute("amount", game_bet.to_string()))
        }
        GameResult::Nought => {
            game.complete_game();
            GAME_MAP.save(deps.storage, game_id.to_string(), &game)?;
            let msg = BankMsg::Send {
                to_address: game.nought.clone().unwrap().to_string(),
                amount: coins(game_bet.u128(), game.bet.denom),
            };
            Ok(res
                .add_message(msg)
                .add_attribute("to", game.nought.clone().unwrap().to_string())
                .add_attribute("amount", game_bet.to_string()))
        }
        GameResult::Draw => {
            game.complete_game();
            GAME_MAP.save(deps.storage, game_id.to_string(), &game)?;
            let g = vec![game.bet];
            Ok(res
                .add_messages(vec![
                    BankMsg::Send {
                        to_address: game.nought.clone().unwrap().to_string(),
                        amount: g.clone(),
                    },
                    BankMsg::Send {
                        to_address: game.cross.to_string(),
                        amount: g,
                    },
                ])
                .add_attributes(vec![
                    ("cross", game.cross.to_string()),
                    ("nought", game.nought.clone().unwrap().to_string()),
                    ("result", String::from("draw")),
                ]))
        }
        GameResult::NoResult => Err(ContractError::Std(StdError::generic_err(
            "Game is not complete",
        ))),
    }
}

pub fn nonpayable(info: &MessageInfo) -> Result<(), ContractError> {
    if info.funds.is_empty() {
        Ok(())
    } else {
        Err(ContractError::Std(StdError::generic_err(
            "does not require coins",
        )))
    }
}

pub fn try_join_game(
    deps: DepsMut,
    info: MessageInfo,
    game_id: Uint128,
) -> Result<Response, ContractError> {
    let mut game = GAME_MAP
        .may_load(deps.storage, game_id.to_string())?
        .unwrap();
    if game.state != GameState::Pending {
        return Err(ContractError::Std(StdError::generic_err(
            "Game is not in Pending state",
        )));
    }
    let is_fund_present = info.funds.iter().any(|funds| funds.eq(&game.bet));
    if !is_fund_present {
        return Err(ContractError::Std(StdError::generic_err(
            "Pass the correct funds",
        )));
    }
    if let Some(_) = game.nought {
        return Err(ContractError::Unauthorized {});
    }
    game.update_opponent(&info.sender);
    game.start_game();
    GAME_MAP.save(deps.storage, game_id.to_string(), &game)?;
    Ok(Response::default().add_attribute("game_id", game_id.to_string()))
}

pub fn try_update_game(
    deps: DepsMut,
    info: MessageInfo,
    game_id: Uint128,
    i: u16,
    j: u16,
    side: bool,
) -> Result<Response, ContractError> {
    nonpayable(&info)?;
    let mut game = GAME_MAP
        .may_load(deps.storage, game_id.to_string())?
        .unwrap();
    if game.state != GameState::Started {
        return Err(ContractError::Std(StdError::generic_err(
            "Game is not started or already finished",
        )));
    }
    if game.next_move != side {
        return Err(ContractError::Std(StdError::generic_err(
            "Next Move and provided Move are not same",
        )));
    }
    if side == true {
        if game.cross != info.sender {
            return Err(ContractError::Std(StdError::GenericErr {
                msg: String::from("sender is not a x"),
            }));
        }
    } else {
        let z_player = game.nought.clone().unwrap();
        if z_player != info.sender {
            return Err(ContractError::Std(StdError::GenericErr {
                msg: String::from("sender is not a 0"),
            }));
        }
    }

    let success = game.update_game(i, j, side);
    if !success {
        return Err(ContractError::Std(StdError::generic_err("illegal move")));
    }
    game.update_side();
    GAME_MAP.save(deps.storage, game_id.to_string(), &game)?;
    Ok(Response::default())
}

pub fn try_create_game(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    bet: Coin,
) -> Result<Response, ContractError> {
    if info.funds.len() != 1 {
        return Err(ContractError::Std(StdError::generic_err(
            "Exactly 1 coin should be present inside the funds",
        )));
    }
    if !info.funds[0].eq(&bet) {
        return Err(ContractError::Std(StdError::generic_err(
            "Funds provided are not equivalent with bet",
        )));
    }
    if GAME_MAP.has(deps.storage, env.block.height.to_string()) {
        return Err(ContractError::Std(StdError::GenericErr {
            msg: String::from("Game is already present at this id"),
        }));
    }
    let game = Game::new(&info.sender, &bet);
    GAME_MAP.save(deps.storage, env.block.height.to_string(), &game)?;
    Ok(Response::default().add_attribute(String::from("id"), env.block.height.to_string()))
}

#[entry_point]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::GetWinner { game_id } => to_binary(&get_winner(deps, game_id)?),
        QueryMsg::QueryGame { game_id } => to_binary(&query_game(deps, game_id)?),
        QueryMsg::AllGames { start_after, limit } => {
            to_binary(&query_all_games(deps, start_after, limit)?)
        }
        QueryMsg::PendingGames {} => to_binary(&query_pending_games(deps)?), // QueryMsg::FindWinnerUsingBoard { game } => to_binary(&find_winner_by_board(game)?),
    }
}

fn query_game(deps: Deps, game_id: Uint128) -> StdResult<Game> {
    let game = GAME_MAP.load(deps.storage, game_id.to_string())?;
    Ok(game)
}

fn get_winner(deps: Deps, game_id: Uint128) -> StdResult<GameResult> {
    let game = GAME_MAP.load(deps.storage, game_id.to_string())?;
    find_winner_by_board(game.game)
}

fn query_pending_games(deps: Deps) -> StdResult<Vec<String>> {
    let game_ids: Result<Vec<_>, _> = GAME_MAP
        .range(deps.storage, None, None, Order::Ascending)
        .filter(|r| return r.as_ref().unwrap().1.state == GameState::Pending)
        .map(|r| r.unwrap().0)
        .map(String::from_utf8)
        .collect();
    Ok(game_ids.unwrap_or(vec![]))
}

fn query_all_games(
    deps: Deps,
    start_after: Option<Uint128>,
    limit: Option<u32>,
) -> StdResult<Vec<String>> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
    let start = start_after.map(|s| Bound::exclusive(s.to_string()));

    let game_ids: Result<Vec<_>, _> = GAME_MAP
        .keys(deps.storage, start, None, Order::Ascending)
        .map(String::from_utf8)
        .take(limit)
        .collect();

    Ok(game_ids.unwrap_or(vec![]))
}

fn find_winner_by_board(game: GameBoard) -> StdResult<GameResult> {
    if !is_valid_board(&game, false) {
        return Err(StdError::generic_err("Please check content of tic tac toe"));
    }
    if validate_game(game.clone(), true)? {
        return Ok(GameResult::Cross);
    }
    if validate_game(game.clone(), false)? {
        return Ok(GameResult::Nought);
    }
    if is_valid_board(&game, true) {
        Ok(GameResult::Draw)
    } else {
        Ok(GameResult::NoResult)
    }
}

fn validate_game(arr: GameBoard, is_cross: bool) -> StdResult<bool> {
    if validate_rows_or_cols(&arr, &is_cross, true) {
        return Ok(true);
    }

    if validate_rows_or_cols(&arr, &is_cross, false) {
        return Ok(true);
    }

    if validate_diagonals(&arr, &is_cross) {
        return Ok(true);
    }

    Ok(false)
}

fn validate_rows_or_cols(arr: &GameBoard, is_cross: &bool, is_row: bool) -> bool {
    for i in 0..3 {
        let mut is_win = true;
        for j in 0..3 {
            let mut val = arr[i][j];
            if !is_row {
                val = arr[j][i];
            }
            match val {
                Some(is_eq_cross) => {
                    if is_eq_cross != *is_cross {
                        is_win = false;
                        break;
                    }
                }
                None => {
                    is_win = false;
                    break;
                }
            }
        }
        if is_win {
            return true;
        }
    }
    false
}

fn validate_diagonals(arr: &GameBoard, is_cross: &bool) -> bool {
    if validate_diagonals_inter(arr, is_cross, false) {
        return true;
    } else {
        if validate_diagonals_inter(arr, is_cross, true) {
            return true;
        }
    }
    false
}

fn validate_diagonals_inter(arr: &GameBoard, is_cross: &bool, is_rev: bool) -> bool {
    for i in 0..3 {
        if is_rev {
            if let Some(is_eq_cross) = arr[i][2 - i] {
                if is_eq_cross != *is_cross {
                    return false;
                }
            } else {
                return false;
            }
        } else {
            if let Some(is_eq_cross) = arr[i][i] {
                if is_eq_cross != *is_cross {
                    return false;
                }
            } else {
                return false;
            }
        }
    }
    true
}

fn is_valid_board(arr: &GameBoard, is_full_mode: bool) -> bool {
    let mut x_count = 0;
    let mut o_count = 0;
    for row in arr.iter() {
        for col in row.iter() {
            if let Some(data) = col {
                if *data {
                    x_count += 1;
                } else {
                    o_count += 1;
                }
            }
        }
    }
    if is_full_mode {
        if x_count == 5 && o_count == 4 {
            true
        } else {
            false
        }
    } else {
        if x_count == 0 || x_count > 5 || o_count == 0 || o_count > 4 {
            false
        } else {
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{mock_dependencies, mock_env, mock_info};
    use cosmwasm_std::{coin, coins, SubMsg};
    #[test]
    fn create_game() {
        let mut deps = mock_dependencies(&coins(2, "token"));

        let cross = String::from("cross");
        let bet = coin(2u128, "cudos");

        let msg = InstantiateMsg {};
        let info = mock_info("creator", &[]);
        let env = mock_env();
        let _res = instantiate(deps.as_mut(), env.clone(), info, msg).unwrap();
        let msg = ExecuteMsg::CreateGame { bet: bet.clone() };
        let info = mock_info(&cross, &[bet.clone()]);
        let env = mock_env();
        let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();
        let d = query_game(deps.as_ref(), Uint128::from(env.block.height)).unwrap();
        matches!(d, Game { .. });
        let bet = coin(2u128, "cudos");
        let msg = ExecuteMsg::CreateGame { bet: bet.clone() };
        let info = mock_info(&cross, &vec![bet]);
        let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
        assert_eq!(
            res,
            ContractError::Std(StdError::generic_err("Game is already present at this id"))
        );
        let bet = coin(2u128, "cudos");
        let msg = ExecuteMsg::CreateGame { bet: bet.clone() };
        let info = mock_info(&cross, &vec![bet.clone(), bet.clone()]);
        let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
        assert_eq!(
            res,
            ContractError::Std(StdError::generic_err(
                "Exactly 1 coin should be present inside the funds"
            ))
        );

        let bet = coin(2u128, "cudos");
        let msg = ExecuteMsg::CreateGame { bet: bet.clone() };
        let info = mock_info(&cross, &coins(1u128, "cudos"));
        let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
        assert_eq!(
            res,
            ContractError::Std(StdError::generic_err(
                "Funds provided are not equivalent with bet",
            ))
        );
    }
    #[test]
    fn join_game() {
        let mut deps = mock_dependencies(&coins(2, "token"));

        let cross = String::from("cross");
        let nought = String::from("nought");
        let bet = coin(2u128, "cudos");

        let msg = InstantiateMsg {};
        let info = mock_info("creator", &[]);
        let env = mock_env();
        let _res = instantiate(deps.as_mut(), env.clone(), info, msg).unwrap();
        let msg = ExecuteMsg::CreateGame { bet: bet.clone() };
        let info = mock_info(&cross, &[bet.clone()]);
        let env = mock_env();
        let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();
        let d = query_game(deps.as_ref(), Uint128::from(env.block.height)).unwrap();
        matches!(d, Game { .. });
        assert_eq!(d.state, GameState::Pending);
        let msg = ExecuteMsg::JoinGame {
            game_id: Uint128::from(env.block.height),
        };
        let info = mock_info(&nought, &[bet.clone()]);
        let env = mock_env();
        let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();
        let d = query_game(deps.as_ref(), Uint128::from(env.block.height)).unwrap();
        matches!(d, Game { .. });
        assert_eq!(d.state, GameState::Started);
    }
    #[test]
    fn update_game() {
        let mut deps = mock_dependencies(&coins(2, "token"));

        let cross = String::from("cross");
        let nought = String::from("nought");
        let bet = coin(2u128, "cudos");

        let msg = InstantiateMsg {};
        let info = mock_info("creator", &[]);
        let env = mock_env();
        let _res = instantiate(deps.as_mut(), env.clone(), info, msg).unwrap();
        let msg = ExecuteMsg::CreateGame { bet: bet.clone() };
        let info = mock_info(&cross, &[bet.clone()]);
        let env = mock_env();
        let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();
        let d = query_game(deps.as_ref(), Uint128::from(env.block.height)).unwrap();
        matches!(d, Game { .. });
        assert_eq!(d.state, GameState::Pending);

        let msg = ExecuteMsg::JoinGame {
            game_id: Uint128::from(env.block.height),
        };
        let info = mock_info(&nought, &[bet.clone()]);
        let env = mock_env();
        let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();
        let d = query_game(deps.as_ref(), Uint128::from(env.block.height)).unwrap();
        matches!(d, Game { .. });
        assert_eq!(d.state, GameState::Started);
        let following: [(&String, bool, u16, u16); 9] = [
            (&cross, true, 0, 0),
            (&nought, false, 2, 0),
            (&cross, true, 0, 2),
            (&nought, false, 1, 0),
            (&cross, true, 1, 2),
            (&nought, false, 1, 1),
            (&cross, true, 2, 2),
            (&nought, false, 2, 1),
            (&cross, true, 0, 1),
        ];
        for (sender, com, i, j) in following {
            let msg = ExecuteMsg::UpdateGame {
                game_id: Uint128::from(env.block.height),
                side: com,
                i: i,
                j: j,
            };
            let info = mock_info(sender, &[]);
            let env = mock_env();
            let _res = execute(deps.as_mut(), env, info, msg).unwrap();
        }
        let msg = ExecuteMsg::UpdateGame {
            game_id: Uint128::from(env.block.height),
            side: false,
            i: 0,
            j: 0,
        };
        let info = mock_info(&nought, &[]);
        let env = mock_env();
        let res = execute(deps.as_mut(), env, info, msg).unwrap_err();
        assert_eq!(
            res,
            ContractError::Std(StdError::generic_err("illegal move"))
        );
    }
    #[test]
    fn withdraw_bet() {
        let mut deps = mock_dependencies(&coins(2, "token"));

        let cross = String::from("cross");
        let nought = String::from("nought");
        let bet = coin(2u128, "cudos");

        let msg = InstantiateMsg {};
        let info = mock_info("creator", &[]);
        let env = mock_env();
        let _res = instantiate(deps.as_mut(), env.clone(), info, msg).unwrap();
        let msg = ExecuteMsg::CreateGame { bet: bet.clone() };
        let info = mock_info(&cross, &[bet.clone()]);
        let env = mock_env();
        let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();
        let d = query_game(deps.as_ref(), Uint128::from(env.block.height)).unwrap();
        matches!(d, Game { .. });
        assert_eq!(d.state, GameState::Pending);

        let msg = ExecuteMsg::JoinGame {
            game_id: Uint128::from(env.block.height),
        };
        let info = mock_info(&nought, &[bet.clone()]);
        let env = mock_env();
        let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();
        let d = query_game(deps.as_ref(), Uint128::from(env.block.height)).unwrap();
        matches!(d, Game { .. });
        assert_eq!(d.state, GameState::Started);
        let following: [(&String, bool, u16, u16); 9] = [
            (&cross, true, 0, 0),
            (&nought, false, 2, 0),
            (&cross, true, 0, 2),
            (&nought, false, 1, 0),
            (&cross, true, 1, 2),
            (&nought, false, 1, 1),
            (&cross, true, 2, 2),
            (&nought, false, 2, 1),
            (&cross, true, 0, 1),
        ];
        for (sender, com, i, j) in following {
            let msg = ExecuteMsg::UpdateGame {
                game_id: Uint128::from(env.block.height),
                side: com,
                i: i,
                j: j,
            };
            let info = mock_info(sender, &[]);
            let env = mock_env();
            let _res = execute(deps.as_mut(), env, info, msg).unwrap();
        }
        let msg = ExecuteMsg::WithdrawBet {
            game_id: Uint128::from(env.block.height),
        };
        let info = mock_info("anyone", &[]);
        let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
        assert_eq!(
            &res.messages[0],
            &SubMsg::new(BankMsg::Send {
                to_address: cross.to_string(),
                amount: coins(4u128, "cudos"),
            })
        );
        let d = query_game(deps.as_ref(), Uint128::from(env.block.height)).unwrap();
        assert_eq!(d.state, GameState::Completed);

        let msg = ExecuteMsg::CreateGame { bet: bet.clone() };
        let info = mock_info(&cross, &[bet.clone()]);
        let mut env = mock_env();
        env.block.height += 100u64;

        let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();
        let d = query_game(deps.as_ref(), Uint128::from(env.block.height)).unwrap();
        matches!(d, Game { .. });
        assert_eq!(d.state, GameState::Pending);

        let msg = ExecuteMsg::JoinGame {
            game_id: Uint128::from(env.block.height),
        };
        let info = mock_info(&nought, &[bet.clone()]);
        let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();
        let following: [(&String, bool, u16, u16); 5] = [
            (&cross, true, 0, 0),
            (&nought, false, 2, 0),
            (&cross, true, 0, 2),
            (&nought, false, 1, 0),
            (&cross, true, 0, 1),
        ];
        for (sender, com, i, j) in following {
            let msg = ExecuteMsg::UpdateGame {
                game_id: Uint128::from(env.block.height),
                side: com,
                i: i,
                j: j,
            };
            let info = mock_info(sender, &[]);
            let env = mock_env();
            let _res = execute(deps.as_mut(), env, info, msg).unwrap();
        }
        let msg = ExecuteMsg::WithdrawBet {
            game_id: Uint128::from(env.block.height),
        };
        let info = mock_info("anyone", &[]);
        let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
        assert_eq!(
            &res.messages[0],
            &SubMsg::new(BankMsg::Send {
                to_address: cross.to_string(),
                amount: coins(4u128, "cudos"),
            })
        );
        let d = query_game(deps.as_ref(), Uint128::from(env.block.height)).unwrap();
        assert_eq!(d.state, GameState::Completed);

        let msg = ExecuteMsg::CreateGame { bet: bet.clone() };
        let info = mock_info(&cross, &[bet.clone()]);
        let mut env = mock_env();
        env.block.height += 1001u64;

        let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();
        let d = query_game(deps.as_ref(), Uint128::from(env.block.height)).unwrap();
        matches!(d, Game { .. });
        assert_eq!(d.state, GameState::Pending);

        let msg = ExecuteMsg::JoinGame {
            game_id: Uint128::from(env.block.height),
        };
        let info = mock_info(&nought, &[bet.clone()]);
        let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();
        let following: [(&String, bool, u16, u16); 4] = [
            (&cross, true, 0, 0),
            (&nought, false, 2, 0),
            (&cross, true, 0, 2),
            (&nought, false, 1, 0),
        ];
        for (sender, com, i, j) in following {
            let msg = ExecuteMsg::UpdateGame {
                game_id: Uint128::from(env.block.height),
                side: com,
                i: i,
                j: j,
            };
            let info = mock_info(sender, &[]);
            let env = mock_env();
            let _res = execute(deps.as_mut(), env, info, msg).unwrap();
        }
        let msg = ExecuteMsg::WithdrawBet {
            game_id: Uint128::from(env.block.height),
        };
        let info = mock_info("anyone", &[]);
        let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap_err();
        assert_eq!(
            res,
            ContractError::Std(StdError::generic_err("Game is not complete"))
        )
    }

    #[test]
    fn pending_game() {
        let mut deps = mock_dependencies(&coins(2, "token"));

        let cross = String::from("cross");
        let nought = String::from("nought");
        let bet = coin(2u128, "cudos");

        let msg = InstantiateMsg {};
        let info = mock_info("creator", &[]);
        let env = mock_env();
        let _res = instantiate(deps.as_mut(), env.clone(), info, msg).unwrap();
        let msg = ExecuteMsg::CreateGame { bet: bet.clone() };
        let info = mock_info(&cross, &[bet.clone()]);
        let env = mock_env();
        let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();
        let d = query_game(deps.as_ref(), Uint128::from(env.block.height)).unwrap();
        matches!(d, Game { .. });
        assert_eq!(d.state, GameState::Pending);

        let msg = ExecuteMsg::CreateGame { bet: bet.clone() };
        let info = mock_info(&cross, &[bet.clone()]);
        let mut env = mock_env();
        env.block.height += 100u64;
        let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();
        let d = query_game(deps.as_ref(), Uint128::from(env.block.height)).unwrap();
        matches!(d, Game { .. });
        assert_eq!(d.state, GameState::Pending);

        let msg = ExecuteMsg::CreateGame { bet: bet.clone() };
        let info = mock_info(&cross, &[bet.clone()]);
        let mut env = mock_env();
        env.block.height += 200u64;
        let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();
        let d = query_game(deps.as_ref(), Uint128::from(env.block.height)).unwrap();
        matches!(d, Game { .. });
        assert_eq!(d.state, GameState::Pending);

        let msg = ExecuteMsg::JoinGame {
            game_id: Uint128::from(env.clone().block.height),
        };
        let info = mock_info(&nought, &[bet.clone()]);
        let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();
        let d = query_game(deps.as_ref(), Uint128::from(env.block.height)).unwrap();
        matches!(d, Game { .. });
        assert_eq!(d.state, GameState::Started);

        let _d = query_pending_games(deps.as_ref()).unwrap();
        matches!(vec!["12345", "12445"], _d);

        let msg = ExecuteMsg::CancelGame {
            game_id: Uint128::from(12345u128),
        };
        let info = mock_info(&cross, &[]);
        let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();
        let d = query_game(deps.as_ref(), Uint128::from(12345u128)).unwrap();
        matches!(d, Game { .. });
        assert_eq!(d.state, GameState::Completed);
        let _d = query_pending_games(deps.as_ref()).unwrap();
        matches!(vec!["12445"], _d);

        let msg = ExecuteMsg::JoinGame {
            game_id: Uint128::from(12345u128),
        };
        let info = mock_info(&nought, &[bet.clone()]);
        let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
        matches!(_res, ContractError::Unauthorized {});
    }
}
