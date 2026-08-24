/** Private Gate 0 result assignments used by the maintained facade. */

export const SET_OUTCOME_CREATED = "created" as const
export const SET_OUTCOME_REPLACED = "replaced" as const
export const SET_OUTCOME_NOT_STORED = "not_stored" as const

export type Gate0_Set_Outcome =
  | typeof SET_OUTCOME_CREATED
  | typeof SET_OUTCOME_REPLACED
