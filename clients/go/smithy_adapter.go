package openkache

import (
	"context"
	"encoding/binary"
	"fmt"
)

// Smithy returns a context-aware adapter implementing the generated Smithy
// service contract. The adapter is useful when an application shares request
// models with another OpenKache language binding.
func (c *Client) Smithy() SmithyOpenKacheAPI {
	return smithyClient{client: c}
}

type smithyClient struct {
	client *Client
}

var _ SmithyOpenKacheAPI = smithyClient{}

func (s smithyClient) Ping(ctx context.Context, _ SmithyPingInput) (SmithyPingOutput, error) {
	return SmithyPingOutput{}, s.client.Ping(ctx)
}

func (s smithyClient) Get(
	ctx context.Context,
	input SmithyGetInput,
) (SmithyGetOutput, error) {
	itemID, err := NewItemID(input.ItemID)
	if err != nil {
		return SmithyGetOutput{}, err
	}
	result, err := s.client.invokeScoped(
		ctx,
		SmithyOpcodeGet,
		input.NamespaceID,
		itemID,
		nil,
		SetOptions{},
	)
	if err != nil {
		return SmithyGetOutput{}, operationError("get", err)
	}
	value, found, err := getResult("get", result)
	if err != nil || !found {
		return SmithyGetOutput{}, err
	}
	return SmithyGetOutput{Value: &value}, nil
}

func (s smithyClient) Set(
	ctx context.Context,
	input SmithySetInput,
) (SmithySetOutput, error) {
	itemID, err := NewItemID(input.ItemID)
	if err != nil {
		return SmithySetOutput{}, err
	}
	options, err := smithySetOptions(input)
	if err != nil {
		return SmithySetOutput{}, err
	}
	result, err := s.client.invokeScoped(
		ctx,
		SmithyOpcodeSet,
		input.NamespaceID,
		itemID,
		input.Value,
		options,
	)
	if err != nil {
		return SmithySetOutput{}, operationError("set", err)
	}
	outcome, err := setResult("set", result)
	return SmithySetOutput{Outcome: SmithySetOutcome(outcome)}, err
}

func (s smithyClient) Delete(
	ctx context.Context,
	input SmithyDeleteInput,
) (SmithyDeleteOutput, error) {
	itemID, err := NewItemID(input.ItemID)
	if err != nil {
		return SmithyDeleteOutput{}, err
	}
	result, err := s.client.invokeScoped(
		ctx,
		SmithyOpcodeDelete,
		input.NamespaceID,
		itemID,
		nil,
		SetOptions{},
	)
	if err != nil {
		return SmithyDeleteOutput{}, operationError("delete", err)
	}
	deleted, err := deleteResult("delete", result)
	return SmithyDeleteOutput{Deleted: deleted}, err
}

func (s smithyClient) Stats(
	ctx context.Context,
	input SmithyStatsInput,
) (SmithyStatsOutput, error) {
	result, err := s.client.invokeScoped(
		ctx,
		SmithyOpcodeStats,
		input.NamespaceID,
		ItemID{},
		nil,
		SetOptions{},
	)
	if err != nil {
		return SmithyStatsOutput{}, operationError("stats", err)
	}
	if result.kind != SmithyFFIResultValue {
		return SmithyStatsOutput{}, unexpectedResult("stats", result.kind)
	}
	return SmithyStatsOutput{JSON: string(result.data)}, nil
}

func (s smithyClient) Sync(
	ctx context.Context,
	input SmithySyncInput,
) (SmithySyncOutput, error) {
	result, err := s.client.invokeScoped(
		ctx,
		SmithyOpcodeSync,
		input.NamespaceID,
		ItemID{},
		nil,
		SetOptions{},
	)
	if err != nil {
		return SmithySyncOutput{}, operationError("sync", err)
	}
	if result.kind != SmithyFFIResultOK {
		return SmithySyncOutput{}, unexpectedResult("sync", result.kind)
	}
	return SmithySyncOutput{}, nil
}

func (s smithyClient) NamespaceOpen(
	ctx context.Context,
	input SmithyNamespaceOpenInput,
) (SmithyNamespaceOpenOutput, error) {
	if input.CreateIfMissing && input.Policy == nil {
		return SmithyNamespaceOpenOutput{}, validationError(
			"namespace.policy",
			"is required when create_if_missing is true",
		)
	}
	if !input.CreateIfMissing && input.Policy != nil {
		return SmithyNamespaceOpenOutput{}, validationError(
			"namespace.policy",
			"is only valid when create_if_missing is true",
		)
	}
	policyFlags, ttl, err := smithyNamespacePolicyWire(input.Policy)
	if err != nil {
		return SmithyNamespaceOpenOutput{}, err
	}
	result, err := s.client.invokeNamespaceOpen(
		ctx,
		[]byte(input.Name),
		input.CreateIfMissing,
		policyFlags,
		ttl,
	)
	if err != nil {
		return SmithyNamespaceOpenOutput{}, operationError("namespace open", err)
	}
	if result.kind != SmithyFFIResultOK && result.kind != SmithyFFIResultCreated {
		return SmithyNamespaceOpenOutput{}, unexpectedResult("namespace open", result.kind)
	}
	descriptor, err := smithyNamespaceDescriptor(result.data)
	if err != nil {
		return SmithyNamespaceOpenOutput{}, err
	}
	return SmithyNamespaceOpenOutput{
		Descriptor: descriptor,
		Created:    result.kind == SmithyFFIResultCreated,
	}, nil
}

func (s smithyClient) NamespaceUpdatePolicy(
	ctx context.Context,
	input SmithyNamespaceUpdatePolicyInput,
) (SmithyNamespaceUpdatePolicyOutput, error) {
	policyFlags, ttl, err := smithyNamespacePolicyWire(&input.Policy)
	if err != nil {
		return SmithyNamespaceUpdatePolicyOutput{}, err
	}
	result, err := s.client.invokeNamespaceUpdatePolicy(
		ctx,
		input.NamespaceID,
		input.ExpectedRevision,
		policyFlags,
		ttl,
	)
	if err != nil {
		return SmithyNamespaceUpdatePolicyOutput{}, operationError(
			"namespace update policy",
			err,
		)
	}
	if result.kind != SmithyFFIResultValue {
		return SmithyNamespaceUpdatePolicyOutput{}, unexpectedResult(
			"namespace update policy",
			result.kind,
		)
	}
	descriptor, err := smithyNamespaceDescriptor(result.data)
	if err != nil {
		return SmithyNamespaceUpdatePolicyOutput{}, err
	}
	return SmithyNamespaceUpdatePolicyOutput{Descriptor: descriptor}, nil
}

func (s smithyClient) NamespaceDelete(
	ctx context.Context,
	input SmithyNamespaceDeleteInput,
) (SmithyNamespaceDeleteOutput, error) {
	result, err := s.client.invokeNamespaceDelete(
		ctx,
		input.NamespaceID,
		input.ExpectedRevision,
	)
	if err != nil {
		return SmithyNamespaceDeleteOutput{}, operationError("namespace delete", err)
	}
	if result.kind != SmithyFFIResultOK {
		return SmithyNamespaceDeleteOutput{}, unexpectedResult("namespace delete", result.kind)
	}
	return SmithyNamespaceDeleteOutput{}, nil
}

func smithySetOptions(input SmithySetInput) (SetOptions, error) {
	if input.ExpirationMode == nil && input.TTLMilliseconds != nil {
		return SetOptions{}, validationError(
			"set.ttl_milliseconds",
			"is only valid with explicit_ttl expiration mode",
		)
	}
	options := SetOptions{}
	if input.Condition != nil {
		options.Condition = *input.Condition
	}
	if input.ExpirationMode != nil {
		options.ExpirationMode = *input.ExpirationMode
	}
	if input.EvictionMode != nil {
		options.EvictionMode = *input.EvictionMode
	}
	if input.TTLMilliseconds != nil {
		options.TTLMillis = *input.TTLMilliseconds
	}
	if err := validateSetOptions(options); err != nil {
		return SetOptions{}, err
	}
	return options, nil
}

func smithyNamespacePolicyWire(
	policy *SmithyNamespacePolicy,
) (uint8, uint64, error) {
	if policy == nil {
		return 0, 0, nil
	}
	var flags uint8
	var ttl uint64
	switch policy.DefaultExpiration {
	case SmithyExpirationDefaultNoExpiry:
		if policy.DefaultTtlMilliseconds != nil {
			return 0, 0, validationError(
				"namespace.policy.default_ttl_milliseconds",
				"is only valid with FixedTtl expiration",
			)
		}
	case SmithyExpirationDefaultFixedTtl:
		if policy.DefaultTtlMilliseconds == nil || *policy.DefaultTtlMilliseconds == 0 {
			return 0, 0, validationError(
				"namespace.policy.default_ttl_milliseconds",
				"must be greater than zero with FixedTtl expiration",
			)
		}
		flags |= uint8(SmithyPolicyFixedTTL)
		ttl = *policy.DefaultTtlMilliseconds
	default:
		return 0, 0, validationError(
			"namespace.policy.default_expiration",
			"contains an unknown value",
		)
	}
	if policy.ExpirationOverride == SmithyOverridePolicyAllowed {
		flags |= uint8(SmithyPolicyExpirationOverride)
	} else if policy.ExpirationOverride != SmithyOverridePolicyDisallowed {
		return 0, 0, validationError(
			"namespace.policy.expiration_override",
			"contains an unknown value",
		)
	}
	switch policy.DefaultEviction {
	case SmithyEvictionDefaultEvictable:
	case SmithyEvictionDefaultEvictionProtected:
		flags |= uint8(SmithyPolicyEvictionProtected)
	default:
		return 0, 0, validationError(
			"namespace.policy.default_eviction",
			"contains an unknown value",
		)
	}
	if policy.EvictionOverride == SmithyOverridePolicyAllowed {
		flags |= uint8(SmithyPolicyEvictionOverride)
	} else if policy.EvictionOverride != SmithyOverridePolicyDisallowed {
		return 0, 0, validationError(
			"namespace.policy.eviction_override",
			"contains an unknown value",
		)
	}
	return flags, ttl, nil
}

func smithyNamespaceDescriptor(payload []byte) (SmithyNamespaceDescriptor, error) {
	const descriptorBytes = SmithyNamespaceIDBytes +
		SmithyNamespaceRevisionBytes +
		SmithyPolicyFlagsBytes +
		SmithyNamespaceRevisionBytes
	if len(payload) != descriptorBytes {
		return SmithyNamespaceDescriptor{}, validationError(
			"namespace.descriptor",
			fmt.Sprintf("must contain exactly %d bytes, got %d", descriptorBytes, len(payload)),
		)
	}
	namespaceID := binary.BigEndian.Uint64(payload[:SmithyNamespaceIDBytes])
	revisionStart := SmithyNamespaceIDBytes
	revision := binary.BigEndian.Uint64(
		payload[revisionStart : revisionStart+SmithyNamespaceRevisionBytes],
	)
	flagsOffset := revisionStart + SmithyNamespaceRevisionBytes
	policy, err := smithyNamespacePolicyFromWire(
		payload[flagsOffset],
		binary.BigEndian.Uint64(payload[flagsOffset+SmithyPolicyFlagsBytes:]),
	)
	if err != nil {
		return SmithyNamespaceDescriptor{}, err
	}
	return SmithyNamespaceDescriptor{
		NamespaceID: namespaceID,
		Revision:    revision,
		Policy:      policy,
	}, nil
}

func smithyNamespacePolicyFromWire(
	flags uint8,
	ttl uint64,
) (SmithyNamespacePolicy, error) {
	if flags&uint8(SmithyPolicyReservedMask) != 0 {
		return SmithyNamespacePolicy{}, validationError(
			"namespace.descriptor.policy",
			"contains reserved flags",
		)
	}
	policy := SmithyNamespacePolicy{
		DefaultExpiration:  SmithyExpirationDefaultNoExpiry,
		ExpirationOverride: SmithyOverridePolicyDisallowed,
		DefaultEviction:    SmithyEvictionDefaultEvictable,
		EvictionOverride:   SmithyOverridePolicyDisallowed,
	}
	switch flags & uint8(SmithyPolicyDefaultExpirationMask) {
	case uint8(SmithyPolicyNoExpiry):
		if ttl != 0 {
			return SmithyNamespacePolicy{}, validationError(
				"namespace.descriptor.policy.default_ttl_milliseconds",
				"must be zero for NoExpiry",
			)
		}
	case uint8(SmithyPolicyFixedTTL):
		if ttl == 0 {
			return SmithyNamespacePolicy{}, validationError(
				"namespace.descriptor.policy.default_ttl_milliseconds",
				"must be positive for FixedTtl",
			)
		}
		policy.DefaultExpiration = SmithyExpirationDefaultFixedTtl
		policy.DefaultTtlMilliseconds = &ttl
	default:
		return SmithyNamespacePolicy{}, validationError(
			"namespace.descriptor.policy.default_expiration",
			"contains an unknown value",
		)
	}
	if flags&uint8(SmithyPolicyExpirationOverride) != 0 {
		policy.ExpirationOverride = SmithyOverridePolicyAllowed
	}
	if flags&uint8(SmithyPolicyEvictionProtected) != 0 {
		policy.DefaultEviction = SmithyEvictionDefaultEvictionProtected
	}
	if flags&uint8(SmithyPolicyEvictionOverride) != 0 {
		policy.EvictionOverride = SmithyOverridePolicyAllowed
	}
	return policy, nil
}
