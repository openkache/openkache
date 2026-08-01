// Code generated from the OpenKache Smithy contract. DO NOT EDIT.

package openkache

import "context"

// SmithySetCondition is the Smithy SetCondition enum.
type SmithySetCondition string

const (
	SmithySetConditionIfAbsent  SmithySetCondition = SmithySetConditionIfAbsentValue
	SmithySetConditionIfPresent SmithySetCondition = SmithySetConditionIfPresentValue
)

// SmithySetOutcome is the Smithy SetOutcome enum.
type SmithySetOutcome string

const (
	SmithySetOutcomeCreated   SmithySetOutcome = SmithySetOutcomeCreatedValue
	SmithySetOutcomeReplaced  SmithySetOutcome = SmithySetOutcomeReplacedValue
	SmithySetOutcomeNotStored SmithySetOutcome = SmithySetOutcomeNotStoredValue
)

// SmithyDeleteInput is the Smithy DeleteInput structure.
type SmithyDeleteInput struct {
	ItemID     []byte  `json:"item_id"`
	MutationID *[]byte `json:"mutation_id,omitempty"`
}

// SmithyDeleteOutput is the Smithy DeleteOutput structure.
type SmithyDeleteOutput struct {
	Deleted bool `json:"deleted"`
}

// SmithyGetInput is the Smithy GetInput structure.
type SmithyGetInput struct {
	ItemID []byte `json:"item_id"`
}

// SmithyGetOutput is the Smithy GetOutput structure.
type SmithyGetOutput struct {
	Value *[]byte `json:"value,omitempty"`
}

// SmithyPingInput is the Smithy PingInput structure.
type SmithyPingInput struct{}

// SmithyPingOutput is the Smithy PingOutput structure.
type SmithyPingOutput struct{}

// SmithySetInput is the Smithy SetInput structure.
type SmithySetInput struct {
	ItemID          []byte              `json:"item_id"`
	Value           []byte              `json:"value"`
	Condition       *SmithySetCondition `json:"condition,omitempty"`
	TTLMilliseconds *int64              `json:"ttl_milliseconds,omitempty"`
	MutationID      *[]byte             `json:"mutation_id,omitempty"`
}

// SmithySetOutput is the Smithy SetOutput structure.
type SmithySetOutput struct {
	Outcome SmithySetOutcome `json:"outcome"`
}

// SmithyStatsInput is the Smithy StatsInput structure.
type SmithyStatsInput struct{}

// SmithyStatsOutput is the Smithy StatsOutput structure.
type SmithyStatsOutput struct {
	JSON string `json:"json"`
}

// SmithySyncInput is the Smithy SyncInput structure.
type SmithySyncInput struct{}

// SmithySyncOutput is the Smithy SyncOutput structure.
type SmithySyncOutput struct{}

// SmithyOpenKacheAPI describes the operations defined by the OpenKache Smithy service.
type SmithyOpenKacheAPI interface {
	Ping(context.Context, SmithyPingInput) (SmithyPingOutput, error)
	Get(context.Context, SmithyGetInput) (SmithyGetOutput, error)
	Set(context.Context, SmithySetInput) (SmithySetOutput, error)
	Delete(context.Context, SmithyDeleteInput) (SmithyDeleteOutput, error)
	Stats(context.Context, SmithyStatsInput) (SmithyStatsOutput, error)
	Sync(context.Context, SmithySyncInput) (SmithySyncOutput, error)
}
