// Code generated from the OpenKache Smithy contract. DO NOT EDIT.

package openkache

import "context"

const (
	// SmithyProtocolALPN is the negotiated protocol identifier.
	SmithyProtocolALPN = "openkache/1"
	// SmithyItemIDBytes is the exact protocol item-ID width.
	SmithyItemIDBytes = 32
	// SmithyMaxValueBytes is the protocol value and payload ceiling.
	SmithyMaxValueBytes = 67108864
	// SmithyDataProtectionKeyBytes is the shared key width.
	SmithyDataProtectionKeyBytes = 32
)

// Smithy operation values carried by the native ABI.
const (
	SmithyOpcodePing uint32 = 1
	SmithyOpcodeGet uint32 = 2
	SmithyOpcodeSet uint32 = 3
	SmithyOpcodeDelete uint32 = 4
	SmithyOpcodeStats uint32 = 5
	SmithyOpcodeSync uint32 = 6
)

// SmithySetCondition is the Smithy SetCondition enum.
type SmithySetCondition string

const (
	SmithySetConditionIfAbsent SmithySetCondition = "if_absent"
	SmithySetConditionIfPresent SmithySetCondition = "if_present"
)

// SmithySetOutcome is the Smithy SetOutcome enum.
type SmithySetOutcome string

const (
	SmithySetOutcomeCreated SmithySetOutcome = "created"
	SmithySetOutcomeReplaced SmithySetOutcome = "replaced"
	SmithySetOutcomeNotStored SmithySetOutcome = "not_stored"
)

// SmithyDeleteInput is the Smithy DeleteInput structure.
type SmithyDeleteInput struct {
	ItemID []byte `json:"item_id"`
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
type SmithyPingInput struct {}

// SmithyPingOutput is the Smithy PingOutput structure.
type SmithyPingOutput struct {}

// SmithySetInput is the Smithy SetInput structure.
type SmithySetInput struct {
	ItemID []byte `json:"item_id"`
	Value []byte `json:"value"`
	Condition *SmithySetCondition `json:"condition,omitempty"`
	TTLMilliseconds *int64 `json:"ttl_milliseconds,omitempty"`
}

// SmithySetOutput is the Smithy SetOutput structure.
type SmithySetOutput struct {
	Outcome SmithySetOutcome `json:"outcome"`
}

// SmithyStatsInput is the Smithy StatsInput structure.
type SmithyStatsInput struct {}

// SmithyStatsOutput is the Smithy StatsOutput structure.
type SmithyStatsOutput struct {
	JSON string `json:"json"`
}

// SmithySyncInput is the Smithy SyncInput structure.
type SmithySyncInput struct {}

// SmithySyncOutput is the Smithy SyncOutput structure.
type SmithySyncOutput struct {}

// SmithyOpenKacheAPI describes the operations defined by the OpenKache Smithy service.
type SmithyOpenKacheAPI interface {
	Ping(context.Context, SmithyPingInput) (SmithyPingOutput, error)
	Get(context.Context, SmithyGetInput) (SmithyGetOutput, error)
	Set(context.Context, SmithySetInput) (SmithySetOutput, error)
	Delete(context.Context, SmithyDeleteInput) (SmithyDeleteOutput, error)
	Stats(context.Context, SmithyStatsInput) (SmithyStatsOutput, error)
	Sync(context.Context, SmithySyncInput) (SmithySyncOutput, error)
}
