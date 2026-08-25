/** Private Gate 0 result assignments used by the maintained facade. */

import {
  SMITHY_SET_OUTCOME_CREATED,
  SMITHY_SET_OUTCOME_NOT_STORED,
  SMITHY_SET_OUTCOME_REPLACED,
  type Smithy_Set_Outcome,
} from "./generated_local/smithy-api.js"

export const SET_OUTCOME_CREATED = SMITHY_SET_OUTCOME_CREATED
export const SET_OUTCOME_REPLACED = SMITHY_SET_OUTCOME_REPLACED
export const SET_OUTCOME_NOT_STORED = SMITHY_SET_OUTCOME_NOT_STORED

export type Gate0_Set_Outcome =
  | typeof SET_OUTCOME_CREATED
  | typeof SET_OUTCOME_REPLACED

type _Gate0_Set_Outcome_Contract_Check = Exclude<
  Smithy_Set_Outcome,
  typeof SET_OUTCOME_CREATED | typeof SET_OUTCOME_REPLACED | typeof SET_OUTCOME_NOT_STORED
> extends never ? true : false

const _gate0_set_outcome_contract_check: _Gate0_Set_Outcome_Contract_Check = true
