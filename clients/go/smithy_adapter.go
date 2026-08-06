package openkache

import "fmt"

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

func smithySetOptions(
	condition *SmithySetCondition,
	expirationMode *SmithyExpirationMode,
	ttlMilliseconds *uint64,
	evictionMode *SmithyEvictionMode,
) (SetOptions, error) {
	if expirationMode == nil && ttlMilliseconds != nil {
		return SetOptions{}, validationError(
			"set.ttl_milliseconds",
			fmt.Sprintf(
				"is only valid with %s expiration mode",
				SmithyExpirationModeExplicitTtlValue,
			),
		)
	}
	options := SetOptions{}
	if condition != nil {
		options.Condition = *condition
	}
	if expirationMode != nil {
		options.ExpirationMode = *expirationMode
	}
	if evictionMode != nil {
		options.EvictionMode = *evictionMode
	}
	if ttlMilliseconds != nil {
		options.TTLMillis = *ttlMilliseconds
	}
	if err := validateSetOptions(options); err != nil {
		return SetOptions{}, err
	}
	return options, nil
}

func smithyNamespacePolicyWire(
	defaultExpiration SmithyExpirationDefault,
	defaultTTLMilliseconds *uint64,
	expirationOverride SmithyOverridePolicy,
	defaultEviction SmithyEvictionDefault,
	evictionOverride SmithyOverridePolicy,
) (uint8, uint64, error) {
	var flags uint8 = uint8(SmithyPolicyNoExpiry)
	var ttl uint64
	switch defaultExpiration {
	case SmithyExpirationDefaultNoExpiry:
		if defaultTTLMilliseconds != nil {
			return 0, 0, validationError(
				"namespace.policy.default_ttl_milliseconds",
				"is only valid with FixedTtl expiration",
			)
		}
	case SmithyExpirationDefaultFixedTtl:
		if defaultTTLMilliseconds == nil || *defaultTTLMilliseconds == 0 {
			return 0, 0, validationError(
				"namespace.policy.default_ttl_milliseconds",
				"must be greater than zero with FixedTtl expiration",
			)
		}
		flags |= uint8(SmithyPolicyFixedTTL)
		ttl = *defaultTTLMilliseconds
	default:
		return 0, 0, validationError(
			"namespace.policy.default_expiration",
			"contains an unknown value",
		)
	}
	if expirationOverride == SmithyOverridePolicyAllowed {
		flags |= uint8(SmithyPolicyExpirationOverride)
	} else if expirationOverride != SmithyOverridePolicyDisallowed {
		return 0, 0, validationError(
			"namespace.policy.expiration_override",
			"contains an unknown value",
		)
	}
	switch defaultEviction {
	case SmithyEvictionDefaultEvictable:
	case SmithyEvictionDefaultEvictionProtected:
		flags |= uint8(SmithyPolicyEvictionProtected)
	default:
		return 0, 0, validationError(
			"namespace.policy.default_eviction",
			"contains an unknown value",
		)
	}
	if evictionOverride == SmithyOverridePolicyAllowed {
		flags |= uint8(SmithyPolicyEvictionOverride)
	} else if evictionOverride != SmithyOverridePolicyDisallowed {
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
