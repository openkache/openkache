// Code generated from the OpenKache Smithy contract. DO NOT EDIT.

package openkache

import "context"

// SmithyEvictionDefault is the Smithy EvictionDefault enum.
type SmithyEvictionDefault string

const (
	SmithyEvictionDefaultEvictable         SmithyEvictionDefault = SmithyEvictionDefaultEvictableValue
	SmithyEvictionDefaultEvictionProtected SmithyEvictionDefault = SmithyEvictionDefaultEvictionProtectedValue
)

// SmithyEvictionMode is the Smithy EvictionMode enum.
type SmithyEvictionMode string

const (
	SmithyEvictionModeInherit           SmithyEvictionMode = SmithyEvictionModeInheritValue
	SmithyEvictionModeEvictable         SmithyEvictionMode = SmithyEvictionModeEvictableValue
	SmithyEvictionModeEvictionProtected SmithyEvictionMode = SmithyEvictionModeEvictionProtectedValue
)

// SmithyExpirationDefault is the Smithy ExpirationDefault enum.
type SmithyExpirationDefault string

const (
	SmithyExpirationDefaultNoExpiry SmithyExpirationDefault = SmithyExpirationDefaultNoExpiryValue
	SmithyExpirationDefaultFixedTtl SmithyExpirationDefault = SmithyExpirationDefaultFixedTtlValue
)

// SmithyExpirationMode is the Smithy ExpirationMode enum.
type SmithyExpirationMode string

const (
	SmithyExpirationModeInherit     SmithyExpirationMode = SmithyExpirationModeInheritValue
	SmithyExpirationModeNoExpiry    SmithyExpirationMode = SmithyExpirationModeNoExpiryValue
	SmithyExpirationModeExplicitTtl SmithyExpirationMode = SmithyExpirationModeExplicitTtlValue
)

// SmithyOverridePolicy is the Smithy OverridePolicy enum.
type SmithyOverridePolicy string

const (
	SmithyOverridePolicyAllowed    SmithyOverridePolicy = SmithyOverridePolicyAllowedValue
	SmithyOverridePolicyDisallowed SmithyOverridePolicy = SmithyOverridePolicyDisallowedValue
)

// SmithySetCondition is the Smithy SetCondition enum.
type SmithySetCondition string

const (
	SmithySetConditionAny       SmithySetCondition = SmithySetConditionAnyValue
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
	NamespaceID uint64 `json:"namespace_id"`
	ItemID      []byte `json:"item_id"`
}

// SmithyDeleteOutput is the Smithy DeleteOutput structure.
type SmithyDeleteOutput struct {
	Deleted bool `json:"deleted"`
}

// SmithyGetInput is the Smithy GetInput structure.
type SmithyGetInput struct {
	NamespaceID uint64 `json:"namespace_id"`
	ItemID      []byte `json:"item_id"`
}

// SmithyGetOutput is the Smithy GetOutput structure.
type SmithyGetOutput struct {
	Value *[]byte `json:"value,omitempty"`
}

// SmithyNamespaceDeleteInput is the Smithy NamespaceDeleteInput structure.
type SmithyNamespaceDeleteInput struct {
	NamespaceID      uint64 `json:"namespace_id"`
	ExpectedRevision uint64 `json:"expected_revision"`
}

// SmithyNamespaceDeleteOutput is the Smithy NamespaceDeleteOutput structure.
type SmithyNamespaceDeleteOutput struct{}

// SmithyNamespaceDescriptor is the Smithy NamespaceDescriptor structure.
type SmithyNamespaceDescriptor struct {
	NamespaceID uint64                `json:"namespace_id"`
	Revision    uint64                `json:"revision"`
	Policy      SmithyNamespacePolicy `json:"policy"`
}

// SmithyNamespaceOpenInput is the Smithy NamespaceOpenInput structure.
type SmithyNamespaceOpenInput struct {
	Name            string                 `json:"name"`
	CreateIfMissing bool                   `json:"create_if_missing"`
	Policy          *SmithyNamespacePolicy `json:"policy,omitempty"`
}

// SmithyNamespaceOpenOutput is the Smithy NamespaceOpenOutput structure.
type SmithyNamespaceOpenOutput struct {
	Descriptor SmithyNamespaceDescriptor `json:"descriptor"`
	Created    bool                      `json:"created"`
}

// SmithyNamespacePolicy is the Smithy NamespacePolicy structure.
type SmithyNamespacePolicy struct {
	DefaultExpiration      SmithyExpirationDefault `json:"default_expiration"`
	DefaultTtlMilliseconds *uint64                 `json:"default_ttl_milliseconds,omitempty"`
	ExpirationOverride     SmithyOverridePolicy    `json:"expiration_override"`
	DefaultEviction        SmithyEvictionDefault   `json:"default_eviction"`
	EvictionOverride       SmithyOverridePolicy    `json:"eviction_override"`
}

// SmithyNamespaceUpdatePolicyInput is the Smithy NamespaceUpdatePolicyInput structure.
type SmithyNamespaceUpdatePolicyInput struct {
	NamespaceID      uint64                `json:"namespace_id"`
	ExpectedRevision uint64                `json:"expected_revision"`
	Policy           SmithyNamespacePolicy `json:"policy"`
}

// SmithyNamespaceUpdatePolicyOutput is the Smithy NamespaceUpdatePolicyOutput structure.
type SmithyNamespaceUpdatePolicyOutput struct {
	Descriptor SmithyNamespaceDescriptor `json:"descriptor"`
}

// SmithyPingInput is the Smithy PingInput structure.
type SmithyPingInput struct{}

// SmithyPingOutput is the Smithy PingOutput structure.
type SmithyPingOutput struct{}

// SmithySetInput is the Smithy SetInput structure.
type SmithySetInput struct {
	NamespaceID     uint64                `json:"namespace_id"`
	ItemID          []byte                `json:"item_id"`
	Value           []byte                `json:"value"`
	Condition       *SmithySetCondition   `json:"condition,omitempty"`
	ExpirationMode  *SmithyExpirationMode `json:"expiration_mode,omitempty"`
	EvictionMode    *SmithyEvictionMode   `json:"eviction_mode,omitempty"`
	TTLMilliseconds *uint64               `json:"ttl_milliseconds,omitempty"`
}

// SmithySetOutput is the Smithy SetOutput structure.
type SmithySetOutput struct {
	Outcome SmithySetOutcome `json:"outcome"`
}

// SmithyStatsInput is the Smithy StatsInput structure.
type SmithyStatsInput struct {
	NamespaceID uint64 `json:"namespace_id"`
}

// SmithyStatsOutput is the Smithy StatsOutput structure.
type SmithyStatsOutput struct {
	JSON string `json:"json"`
}

// SmithySyncInput is the Smithy SyncInput structure.
type SmithySyncInput struct {
	NamespaceID uint64 `json:"namespace_id"`
}

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
	NamespaceOpen(context.Context, SmithyNamespaceOpenInput) (SmithyNamespaceOpenOutput, error)
	NamespaceUpdatePolicy(context.Context, SmithyNamespaceUpdatePolicyInput) (SmithyNamespaceUpdatePolicyOutput, error)
	NamespaceDelete(context.Context, SmithyNamespaceDeleteInput) (SmithyNamespaceDeleteOutput, error)
}
