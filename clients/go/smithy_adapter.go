package openkache

import (
	"context"
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
	decoded, err := s.client.decodeNamespaceDescriptor(ctx, result.data)
	if err != nil {
		return SmithyNamespaceOpenOutput{}, err
	}
	return SmithyNamespaceOpenOutput{
		Descriptor: smithyNamespaceDescriptor(decoded),
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
	decoded, err := s.client.decodeNamespaceDescriptor(ctx, result.data)
	if err != nil {
		return SmithyNamespaceUpdatePolicyOutput{}, err
	}
	return SmithyNamespaceUpdatePolicyOutput{
		Descriptor: smithyNamespaceDescriptor(decoded),
	}, nil
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
			fmt.Sprintf(
				"is only valid with %s expiration mode",
				SmithyExpirationModeExplicitTtlValue,
			),
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
	var flags uint8 = uint8(SmithyPolicyNoExpiry)
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

func smithyNamespaceDescriptor(
	decoded nativeNamespaceDescriptor,
) SmithyNamespaceDescriptor {
	defaultExpiration := SmithyExpirationDefaultNoExpiry
	var defaultTTL *uint64
	if decoded.DefaultExpiration == SmithyFFINamespaceDefaultExpirationFixedTtl {
		defaultExpiration = SmithyExpirationDefaultFixedTtl
		ttl := decoded.DefaultTtlMs
		defaultTTL = &ttl
	}
	return SmithyNamespaceDescriptor{
		NamespaceID: decoded.NamespaceID,
		Revision:    decoded.Revision,
		Policy: SmithyNamespacePolicy{
			DefaultExpiration:      defaultExpiration,
			DefaultTtlMilliseconds: defaultTTL,
			ExpirationOverride:     smithyOverridePolicy(decoded.ExpirationOverride),
			DefaultEviction:        smithyEvictionDefault(decoded.DefaultEviction),
			EvictionOverride:       smithyOverridePolicy(decoded.EvictionOverride),
		},
	}
}

func smithyOverridePolicy(value uint32) SmithyOverridePolicy {
	if value == SmithyFFINamespaceOverrideAllowed {
		return SmithyOverridePolicyAllowed
	}
	return SmithyOverridePolicyDisallowed
}

func smithyEvictionDefault(value uint32) SmithyEvictionDefault {
	if value == SmithyFFINamespaceDefaultEvictionProtected {
		return SmithyEvictionDefaultEvictionProtected
	}
	return SmithyEvictionDefaultEvictable
}
