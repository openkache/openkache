package openkache

import (
	"bytes"
	"context"
	"encoding/pem"
	"errors"
	"fmt"
	"runtime"
	"sync"
	"time"
)

const maxCanonicalKeyBytes = 1 << 20

// ErrClosed is returned after a client has been permanently closed.
var ErrClosed = errors.New("openkache: client is closed")

// Error is a failure returned by the shared native client.
type Error struct {
	// Operation identifies the Go operation that observed the failure.
	Operation string
	// Message is the native or validation detail.
	Message string
	// Cause is an optional underlying context or validation error.
	Cause error
}

func (e *Error) Error() string {
	if e.Operation == "" {
		return e.Message
	}
	if e.Message == "" {
		return "openkache " + e.Operation + " failed"
	}
	return "openkache " + e.Operation + " failed: " + e.Message
}

func (e *Error) Unwrap() error {
	return e.Cause
}

// Identity contains the certificate chain and private key for mutual TLS.
//
// Each certificate and the private key may be DER or PEM. PEM certificate
// chains are concatenated in order before being passed to the shared core.
type Identity struct {
	CertificateChain [][]byte
	PrivateKey       []byte
}

// CompressionOptions controls optional Zstandard compression in the core.
type CompressionOptions struct {
	// Enabled enables Zstandard when true.
	Enabled bool
	// Level is the Zstandard level from the shared contract range. Zero selects
	// the shared default level.
	Level int32
	// MinimumInputSize skips compression below this many bytes. Zero selects
	// the shared default.
	MinimumInputSize int
	// MinimumSavings requires a compressed frame to save this many bytes. Zero
	// selects the shared default.
	MinimumSavings int
}

// Encryption selects the shared-core value-protection profile.
//
// Compact is deterministic for a given item ID. Robust is randomized and is
// the default profile used by the protected client.
type Encryption uint32

const (
	// EncryptionCompact selects AES-256-SIV-CMAC.
	EncryptionCompact Encryption = Encryption(SmithyValueEncryptionCompact)
	// EncryptionRobust selects AES-256-GCM-SIV.
	EncryptionRobust Encryption = Encryption(SmithyValueEncryptionRobust)
)

// TimeoutOptions bounds connection setup and complete request exchanges.
type TimeoutOptions struct {
	Connect time.Duration
	Request time.Duration
}

// RetryPolicy controls retries for response-safe operations.
type RetryPolicy struct {
	// MaxAttempts includes the initial attempt. Zero selects the shared default.
	MaxAttempts int
}

// Options configures a protected OpenKache connection.
type Options struct {
	// Address is the server host and UDP port, such as "127.0.0.1:4433" or
	// "cache.example.com:4433".
	Address string
	// ServerName is the TLS certificate name. Empty selects the shared default.
	ServerName string
	// Certificate is one DER certificate or a PEM certificate chain trusted for
	// the server connection. Empty selects the platform/system trust roots.
	Certificate []byte
	// Identity is optional mutual-TLS client authentication material.
	Identity *Identity
	// DataProtectionKey is the persistent application data-protection secret.
	DataProtectionKey []byte
	// Compression is applied before the core's authenticated encryption.
	Compression CompressionOptions
	// Encryption selects the shared-core value-protection profile. The zero
	// value selects EncryptionRobust.
	Encryption Encryption
	// Timeouts bounds native connection and operation work.
	Timeouts TimeoutOptions
	// Retry controls response-safe retry attempts.
	Retry RetryPolicy
	// MaxInFlight bounds reusable QUIC stream lanes. Zero selects the shared
	// default.
	MaxInFlight int
	// NativeLibrary overrides native library discovery. The default consults
	// OPENKACHE_CLIENT_LIBRARY and then platform library names.
	NativeLibrary string
}

type normalizedOptions struct {
	address             string
	serverName          string
	certificate         []byte
	identityCertificate []byte
	identityPrivateKey  []byte
	dataProtectionKey   []byte
	compression         CompressionOptions
	encryption          Encryption
	timeouts            TimeoutOptions
	retryAttempts       int
	maxInFlight         int
	nativeLibrary       string
}

func (o Options) normalize() (normalizedOptions, error) {
	if o.Address == "" {
		return normalizedOptions{}, validationError("address", "must not be empty")
	}
	if len(o.DataProtectionKey) != 0 && len(o.DataProtectionKey) != SmithyDataProtectionKeyBytes {
		return normalizedOptions{}, validationError(
			"data_protection_key",
			fmt.Sprintf("must contain exactly %d bytes, got %d", SmithyDataProtectionKeyBytes, len(o.DataProtectionKey)),
		)
	}

	serverName := o.ServerName
	if serverName == "" {
		serverName = SmithyClientDefaultServerName
	}

	connectTimeout := o.Timeouts.Connect
	if connectTimeout == 0 {
		connectTimeout = time.Duration(SmithyDefaultConnectTimeoutMilliseconds) * time.Millisecond
	}
	requestTimeout := o.Timeouts.Request
	if requestTimeout == 0 {
		requestTimeout = time.Duration(SmithyDefaultRequestTimeoutMilliseconds) * time.Millisecond
	}
	minimumTimeout := time.Duration(SmithyClientMinimumPositiveValue) * time.Millisecond
	if connectTimeout < 0 || requestTimeout < 0 {
		return normalizedOptions{}, validationError("timeouts", "must not be negative")
	}
	if connectTimeout < minimumTimeout || requestTimeout < minimumTimeout {
		return normalizedOptions{}, validationError("timeouts", "must be at least one millisecond")
	}

	compression := o.Compression
	if compression.MinimumInputSize < 0 || compression.MinimumSavings < 0 {
		return normalizedOptions{}, validationError("compression", "size thresholds must not be negative")
	}
	if compression.Enabled {
		if compression.Level == 0 {
			compression.Level = SmithyDefaultZstandardLevel
		}
		if compression.Level < SmithyDefaultZstandardLevelMin ||
			compression.Level > SmithyDefaultZstandardLevelMax {
			return normalizedOptions{}, validationError(
				"compression.level",
				fmt.Sprintf(
					"must be from %d through %d",
					SmithyDefaultZstandardLevelMin,
					SmithyDefaultZstandardLevelMax,
				),
			)
		}
		if compression.MinimumInputSize == 0 {
			compression.MinimumInputSize = SmithyDefaultZstandardMinimumInputBytes
		}
		if compression.MinimumSavings == 0 {
			compression.MinimumSavings = SmithyDefaultZstandardMinimumSavingsBytes
		}
	}

	encryption := o.Encryption
	if encryption == 0 {
		encryption = EncryptionRobust
	}
	if encryption != EncryptionCompact && encryption != EncryptionRobust {
		return normalizedOptions{}, validationError("encryption", "must be EncryptionCompact or EncryptionRobust")
	}

	retryAttempts := o.Retry.MaxAttempts
	if retryAttempts == 0 {
		retryAttempts = SmithyDefaultRetryMaxAttempts
	}
	if retryAttempts < SmithyClientMinimumPositiveValue {
		return normalizedOptions{}, validationError("retry.max_attempts", "must be greater than zero")
	}
	maxInFlight := o.MaxInFlight
	if maxInFlight == 0 {
		maxInFlight = SmithyDefaultMaxInFlight
	}
	if maxInFlight < SmithyClientMinimumPositiveValue {
		return normalizedOptions{}, validationError("max_in_flight", "must be greater than zero")
	}

	var identityCertificate []byte
	var identityPrivateKey []byte
	if o.Identity != nil {
		if len(o.Identity.CertificateChain) == 0 {
			return normalizedOptions{}, validationError(
				"identity.certificate_chain",
				"must not be empty",
			)
		}
		if len(o.Identity.PrivateKey) == 0 {
			return normalizedOptions{}, validationError("identity.private_key", "must not be empty")
		}
		for index, certificate := range o.Identity.CertificateChain {
			if len(certificate) == 0 {
				return normalizedOptions{}, validationError(
					"identity.certificate_chain",
					fmt.Sprintf("certificate %d must not be empty", index),
				)
			}
			trimmed := bytes.TrimSpace(certificate)
			if isPEM(trimmed) {
				identityCertificate = append(identityCertificate, trimmed...)
				identityCertificate = append(identityCertificate, '\n')
			} else if len(o.Identity.CertificateChain) == 1 {
				identityCertificate = append(identityCertificate, certificate...)
			} else {
				identityCertificate = append(
					identityCertificate,
					pem.EncodeToMemory(
						&pem.Block{Type: SmithyClientCertificatePEMType, Bytes: certificate},
					)...,
				)
			}
		}
		identityPrivateKey = normalizedPEM(o.Identity.PrivateKey)
	}

	return normalizedOptions{
		address:             o.Address,
		serverName:          serverName,
		certificate:         normalizedPEM(o.Certificate),
		identityCertificate: identityCertificate,
		identityPrivateKey:  identityPrivateKey,
		dataProtectionKey:   append([]byte(nil), o.DataProtectionKey...),
		compression:         compression,
		encryption:          encryption,
		timeouts:            TimeoutOptions{Connect: connectTimeout, Request: requestTimeout},
		retryAttempts:       retryAttempts,
		maxInFlight:         maxInFlight,
		nativeLibrary:       o.NativeLibrary,
	}, nil
}

func normalizedPEM(value []byte) []byte {
	trimmed := bytes.TrimSpace(value)
	if isPEM(trimmed) {
		return append([]byte(nil), trimmed...)
	}
	return append([]byte(nil), value...)
}

func isPEM(value []byte) bool {
	block, _ := pem.Decode(value)
	return block != nil
}

func validationError(field, message string) error {
	return &Error{Operation: "configuration", Message: field + ": " + message}
}

// SetCondition is the atomic existence condition for a SET operation.
type SetCondition = SmithySetCondition

const (
	// IfAbsent stores only when the key is absent.
	IfAbsent SetCondition = SmithySetConditionIfAbsent
	// IfPresent stores only when the key is present.
	IfPresent SetCondition = SmithySetConditionIfPresent
)

// SetOptions controls an individual SET operation.
type SetOptions struct {
	// Condition selects the atomic existence predicate. The zero value is
	// unconditional.
	Condition SetCondition
	// ExpirationMode selects whether the item inherits the namespace policy,
	// never expires, or uses TTLMillis. The zero value inherits.
	ExpirationMode SmithyExpirationMode
	// EvictionMode selects whether the item inherits the namespace policy,
	// remains evictable, or is protected from capacity eviction. The zero value
	// inherits.
	EvictionMode SmithyEvictionMode
	// TTLMillis is the positive relative lifetime used by ExplicitTtl. For
	// compatibility, a non-zero value with an empty ExpirationMode selects
	// ExplicitTtl.
	TTLMillis uint64
}

// ConnectionState is a best-effort snapshot of the native connection state.
type ConnectionState uint32

const (
	// ConnectionStateConnected means the latest native connection is available.
	ConnectionStateConnected ConnectionState = ConnectionState(SmithyFFIConnectionStateConnected)
	// ConnectionStateReconnecting means a request is replacing a failed connection.
	ConnectionStateReconnecting ConnectionState = ConnectionState(SmithyFFIConnectionStateReconnecting)
	// ConnectionStateDisconnected means a later operation will reconnect.
	ConnectionStateDisconnected ConnectionState = ConnectionState(SmithyFFIConnectionStateDisconnected)
	// ConnectionStateClosed means the client was permanently closed.
	ConnectionStateClosed ConnectionState = ConnectionState(SmithyFFIConnectionStateClosed)
	// ConnectionStateUnknown means the native library cannot report a known state.
	ConnectionStateUnknown ConnectionState = ConnectionState(SmithyFFIConnectionStateUnknown)
)

// String returns the stable textual connection-state name.
func (state ConnectionState) String() string {
	switch state {
	case ConnectionStateConnected:
		return "Connected"
	case ConnectionStateReconnecting:
		return "Reconnecting"
	case ConnectionStateDisconnected:
		return "Disconnected"
	case ConnectionStateClosed:
		return "Closed"
	case ConnectionStateUnknown:
		return "Unknown"
	default:
		return "Unknown"
	}
}

// ItemID is the exact fixed-width identifier carried by the wire protocol.
type ItemID [SmithyItemIDBytes]byte

// NewItemID copies an exact protocol-width wire item ID.
func NewItemID(value []byte) (ItemID, error) {
	var itemID ItemID
	if len(value) != SmithyItemIDBytes {
		return itemID, validationError(
			"item_id",
			fmt.Sprintf("must contain exactly %d bytes, got %d", SmithyItemIDBytes, len(value)),
		)
	}
	copy(itemID[:], value)
	return itemID, nil
}

// Bytes returns a copy of the exact item-ID bytes.
func (id ItemID) Bytes() []byte {
	value := make([]byte, len(id))
	copy(value, id[:])
	return value
}

// SetOutcome is the result of a successful SET operation.
type SetOutcome = SmithySetOutcome

const (
	// Created means a new item was stored.
	Created SetOutcome = SmithySetOutcomeCreated
	// Replaced means an existing item was replaced.
	Replaced SetOutcome = SmithySetOutcomeReplaced
	// NotStored means the existence condition did not match.
	NotStored SetOutcome = SmithySetOutcomeNotStored
)

type nativeResult struct {
	kind uint32
	data []byte
}

type nativeNamespaceDescriptor = SmithyFFINamespaceDescriptor

type nativeClient interface {
	execute(context.Context, uint32, []byte, []byte, SetOptions) (nativeResult, error)
	executeRaw(context.Context, uint32, ItemID, []byte, SetOptions) (nativeResult, error)
	executeScoped(
		context.Context,
		uint32,
		uint64,
		ItemID,
		[]byte,
		SetOptions,
	) (nativeResult, error)
	executeScopedBytes(
		context.Context,
		uint32,
		uint64,
		[]byte,
		[]byte,
		SetOptions,
	) (nativeResult, error)
	namespaceOpen(
		context.Context,
		[]byte,
		bool,
		uint8,
		uint64,
	) (nativeResult, error)
	namespaceUpdatePolicy(
		context.Context,
		uint64,
		uint64,
		uint8,
		uint64,
	) (nativeResult, error)
	namespaceDelete(context.Context, uint64, uint64) (nativeResult, error)
	decodeNamespaceDescriptor([]byte) (nativeNamespaceDescriptor, error)
	state() uint32
	close() error
}

// Client is a concurrency-safe protected OpenKache client.
//
// A client may be used by multiple goroutines. Close is permanent and waits
// for native calls already in flight before releasing the native library.
type Client struct {
	mu     sync.RWMutex
	native nativeClient
}

// Connect validates options and establishes a protected OpenKache connection.
func Connect(ctx context.Context, options Options) (*Client, error) {
	if ctx == nil {
		return nil, validationError("context", "must not be nil")
	}
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	normalized, err := options.normalize()
	if err != nil {
		return nil, err
	}
	native, err := connectNative(ctx, normalized)
	if err != nil {
		return nil, err
	}
	client := &Client{native: native}
	runtime.SetFinalizer(client, (*Client).finalize)
	return client, nil
}

func (c *Client) finalize() {
	_ = c.Close()
}

func (c *Client) invoke(
	ctx context.Context,
	operation uint32,
	key, value []byte,
	options SetOptions,
) (nativeResult, error) {
	return c.invokeNative(ctx, func(native nativeClient) (nativeResult, error) {
		return native.execute(ctx, operation, key, value, options)
	})
}

func (c *Client) invokeRaw(
	ctx context.Context,
	operation uint32,
	itemID ItemID,
	value []byte,
	options SetOptions,
) (nativeResult, error) {
	return c.invokeNative(ctx, func(native nativeClient) (nativeResult, error) {
		return native.executeRaw(ctx, operation, itemID, value, options)
	})
}

func (c *Client) invokeScoped(
	ctx context.Context,
	operation uint32,
	namespaceID uint64,
	itemID ItemID,
	value []byte,
	options SetOptions,
) (nativeResult, error) {
	return c.invokeNative(ctx, func(native nativeClient) (nativeResult, error) {
		return native.executeScoped(ctx, operation, namespaceID, itemID, value, options)
	})
}

func (c *Client) invokeScopedBytes(
	ctx context.Context,
	operation uint32,
	namespaceID uint64,
	itemID []byte,
	value []byte,
	options SetOptions,
) (nativeResult, error) {
	return c.invokeNative(ctx, func(native nativeClient) (nativeResult, error) {
		return native.executeScopedBytes(ctx, operation, namespaceID, itemID, value, options)
	})
}

func (c *Client) invokeNamespaceOpen(
	ctx context.Context,
	name []byte,
	createIfMissing bool,
	policyFlags uint8,
	ttl uint64,
) (nativeResult, error) {
	return c.invokeNative(ctx, func(native nativeClient) (nativeResult, error) {
		return native.namespaceOpen(ctx, name, createIfMissing, policyFlags, ttl)
	})
}

func (c *Client) invokeNamespaceUpdatePolicy(
	ctx context.Context,
	namespaceID uint64,
	expectedRevision uint64,
	policyFlags uint8,
	ttl uint64,
) (nativeResult, error) {
	return c.invokeNative(ctx, func(native nativeClient) (nativeResult, error) {
		return native.namespaceUpdatePolicy(
			ctx,
			namespaceID,
			expectedRevision,
			policyFlags,
			ttl,
		)
	})
}

func (c *Client) invokeNamespaceDelete(
	ctx context.Context,
	namespaceID uint64,
	expectedRevision uint64,
) (nativeResult, error) {
	return c.invokeNative(ctx, func(native nativeClient) (nativeResult, error) {
		return native.namespaceDelete(ctx, namespaceID, expectedRevision)
	})
}

func (c *Client) decodeNamespaceDescriptor(
	ctx context.Context,
	payload []byte,
) (nativeNamespaceDescriptor, error) {
	if ctx == nil {
		return nativeNamespaceDescriptor{}, validationError("context", "must not be nil")
	}
	if err := ctx.Err(); err != nil {
		return nativeNamespaceDescriptor{}, err
	}
	c.mu.RLock()
	defer c.mu.RUnlock()
	native := c.native
	if native == nil {
		return nativeNamespaceDescriptor{}, ErrClosed
	}
	return native.decodeNamespaceDescriptor(payload)
}

func (c *Client) invokeNative(
	ctx context.Context,
	invoke func(nativeClient) (nativeResult, error),
) (nativeResult, error) {
	if ctx == nil {
		return nativeResult{}, validationError("context", "must not be nil")
	}
	if err := ctx.Err(); err != nil {
		return nativeResult{}, err
	}
	c.mu.RLock()
	defer c.mu.RUnlock()
	native := c.native
	if native == nil {
		return nativeResult{}, ErrClosed
	}
	return invoke(native)
}

// Ping verifies the connection and the server's PONG response.
func (c *Client) Ping(ctx context.Context) error {
	result, err := c.invoke(ctx, SmithyOpcodePing, nil, nil, SetOptions{})
	if err != nil {
		return operationError("ping", err)
	}
	if result.kind != SmithyFFIResultOK {
		return unexpectedResult("ping", result.kind)
	}
	return nil
}

// Get retrieves decrypted and decompressed bytes for key. The found result is
// distinguished from an empty stored value by the found boolean.
func (c *Client) Get(ctx context.Context, key []byte) ([]byte, bool, error) {
	canonicalKey, err := canonicalBytesKey(key)
	if err != nil {
		return nil, false, err
	}
	result, err := c.invoke(ctx, SmithyOpcodeGet, canonicalKey, nil, SetOptions{})
	if err != nil {
		return nil, false, operationError("get", err)
	}
	return getResult("get", result)
}

// GetJSON retrieves the canonical JSON document stored for key.
//
// The returned bytes are canonical RFC 8785 JSON produced by the shared core.
// The Go adapter does not parse or re-serialize the document.
func (c *Client) GetJSON(ctx context.Context, key []byte) ([]byte, bool, error) {
	canonicalKey, err := canonicalBytesKey(key)
	if err != nil {
		return nil, false, err
	}
	result, err := c.invoke(ctx, SmithyFFIOperationGetJson, canonicalKey, nil, SetOptions{})
	if err != nil {
		return nil, false, operationError("get json", err)
	}
	return getResult("get json", result)
}

// GetItem retrieves an exact wire item ID without application-key derivation or
// value protection.
func (c *Client) GetItem(ctx context.Context, itemID ItemID) ([]byte, bool, error) {
	result, err := c.invokeRaw(ctx, SmithyOpcodeGet, itemID, nil, SetOptions{})
	if err != nil {
		return nil, false, operationError("get item", err)
	}
	return getResult("get item", result)
}

// Set encrypts and stores value for key.
func (c *Client) Set(ctx context.Context, key, value []byte, options SetOptions) (SetOutcome, error) {
	canonicalKey, err := canonicalBytesKey(key)
	if err != nil {
		return "", err
	}
	if len(value) > SmithyMaxValueBytes {
		return "", validationError("value", fmt.Sprintf("exceeds %d bytes", SmithyMaxValueBytes))
	}
	if err := validateSetOptions(options); err != nil {
		return "", err
	}
	result, err := c.invoke(ctx, SmithyOpcodeSet, canonicalKey, value, options)
	if err != nil {
		return "", operationError("set", err)
	}
	return setResult("set", result)
}

// SetJSON stores one complete JSON document for key.
//
// The shared core parses and canonicalizes the document before serialization;
// callers must not pre-serialize native Go values with encoding/json when
// cross-language canonical bytes matter.
func (c *Client) SetJSON(
	ctx context.Context,
	key, jsonBytes []byte,
	options SetOptions,
) (SetOutcome, error) {
	canonicalKey, err := canonicalBytesKey(key)
	if err != nil {
		return "", err
	}
	if len(jsonBytes) > SmithyMaxValueBytes {
		return "", validationError(
			"json",
			fmt.Sprintf("exceeds %d bytes", SmithyMaxValueBytes),
		)
	}
	if err := validateSetOptions(options); err != nil {
		return "", err
	}
	result, err := c.invoke(ctx, SmithyFFIOperationSetJson, canonicalKey, jsonBytes, options)
	if err != nil {
		return "", operationError("set json", err)
	}
	return setResult("set json", result)
}

func validateSetOptions(options SetOptions) error {
	if options.Condition != "" && options.Condition != SmithySetConditionAny &&
		options.Condition != IfAbsent && options.Condition != IfPresent {
		return validationError(
			"set.condition",
			"must be empty, Any, IfAbsent, or IfPresent",
		)
	}
	if options.TTLMillis == 0 &&
		options.ExpirationMode == SmithyExpirationModeExplicitTtl {
		return validationError(
			"set.ttl_milliseconds",
			"must be greater than zero with ExplicitTtl expiration",
		)
	}
	switch options.ExpirationMode {
	case "":
		// A non-zero TTL is the legacy shorthand for ExplicitTtl.
		if options.TTLMillis != 0 {
			break
		}
	case SmithyExpirationModeInherit, SmithyExpirationModeNoExpiry:
		if options.TTLMillis != 0 {
			return validationError(
				"set.ttl_milliseconds",
				"is only valid with ExplicitTtl expiration",
			)
		}
	case SmithyExpirationModeExplicitTtl:
		if options.TTLMillis == 0 {
			return validationError(
				"set.ttl_milliseconds",
				"must be greater than zero with ExplicitTtl expiration",
			)
		}
	default:
		return validationError("set.expiration_mode", "contains an unknown value")
	}
	switch options.EvictionMode {
	case "", SmithyEvictionModeInherit, SmithyEvictionModeEvictable,
		SmithyEvictionModeEvictionProtected:
	default:
		return validationError("set.eviction_mode", "contains an unknown value")
	}
	return nil
}

func (options SetOptions) wireFlags() (uint8, uint64, error) {
	if err := validateSetOptions(options); err != nil {
		return 0, 0, err
	}
	flags := uint8(SmithySetConditionAnyBits)
	switch options.Condition {
	case "", SmithySetConditionAny:
	case SmithySetConditionIfAbsent:
		flags |= uint8(SmithySetIfAbsentBits)
	case SmithySetConditionIfPresent:
		flags |= uint8(SmithySetIfPresentBits)
	default:
		return 0, 0, validationError("set.condition", "contains an unknown value")
	}

	expiration := options.ExpirationMode
	if expiration == "" {
		if options.TTLMillis != 0 {
			expiration = SmithyExpirationModeExplicitTtl
		} else {
			expiration = SmithyExpirationModeInherit
		}
	}
	switch expiration {
	case SmithyExpirationModeInherit:
		flags |= uint8(SmithySetInheritExpirationBits)
	case SmithyExpirationModeNoExpiry:
		flags |= uint8(SmithySetNoExpiryBits)
	case SmithyExpirationModeExplicitTtl:
		flags |= uint8(SmithySetExplicitTTLBits)
	default:
		return 0, 0, validationError("set.expiration_mode", "contains an unknown value")
	}

	switch options.EvictionMode {
	case "", SmithyEvictionModeInherit:
		flags |= uint8(SmithySetInheritEvictionBits)
	case SmithyEvictionModeEvictable:
		flags |= uint8(SmithySetEvictableBits)
	case SmithyEvictionModeEvictionProtected:
		flags |= uint8(SmithySetEvictionProtectedBits)
	default:
		return 0, 0, validationError("set.eviction_mode", "contains an unknown value")
	}
	if expiration != SmithyExpirationModeExplicitTtl {
		return flags, 0, nil
	}
	return flags, options.TTLMillis, nil
}

// SetItem stores exact opaque bytes for a wire item ID.
func (c *Client) SetItem(
	ctx context.Context,
	itemID ItemID,
	value []byte,
	options SetOptions,
) (SetOutcome, error) {
	if len(value) > SmithyMaxValueBytes {
		return "", validationError("value", fmt.Sprintf("exceeds %d bytes", SmithyMaxValueBytes))
	}
	if err := validateSetOptions(options); err != nil {
		return "", err
	}
	result, err := c.invokeRaw(ctx, SmithyOpcodeSet, itemID, value, options)
	if err != nil {
		return "", operationError("set item", err)
	}
	return setResult("set item", result)
}

// Delete removes key and reports whether an item existed.
func (c *Client) Delete(ctx context.Context, key []byte) (bool, error) {
	canonicalKey, err := canonicalBytesKey(key)
	if err != nil {
		return false, err
	}
	result, err := c.invoke(ctx, SmithyOpcodeDelete, canonicalKey, nil, SetOptions{})
	if err != nil {
		return false, operationError("delete", err)
	}
	return deleteResult("delete", result)
}

// canonicalBytesKey encodes a Go []byte key as the v1 Bytes PortableKey.
// The native ABI accepts canonical key bytes, not the caller's raw bytes.
func canonicalBytesKey(key []byte) ([]byte, error) {
	header, err := cborArgument(2, uint64(len(key)))
	if err != nil {
		return nil, err
	}
	if len(header)+len(key) > maxCanonicalKeyBytes {
		return nil, validationError(
			"key",
			fmt.Sprintf("canonical encoding exceeds %d bytes", maxCanonicalKeyBytes),
		)
	}
	encoded := make([]byte, 0, len(header)+len(key))
	encoded = append(encoded, header...)
	encoded = append(encoded, key...)
	return encoded, nil
}

func cborArgument(major byte, value uint64) ([]byte, error) {
	if major > 7 {
		return nil, validationError("key", "invalid CBOR major type")
	}
	prefix := major << 5
	switch {
	case value <= 23:
		return []byte{prefix | byte(value)}, nil
	case value <= 0xff:
		return []byte{prefix | 24, byte(value)}, nil
	case value <= 0xffff:
		return []byte{prefix | 25, byte(value >> 8), byte(value)}, nil
	case value <= 0xffff_ffff:
		return []byte{
			prefix | 26,
			byte(value >> 24),
			byte(value >> 16),
			byte(value >> 8),
			byte(value),
		}, nil
	default:
		return []byte{
			prefix | 27,
			byte(value >> 56),
			byte(value >> 48),
			byte(value >> 40),
			byte(value >> 32),
			byte(value >> 24),
			byte(value >> 16),
			byte(value >> 8),
			byte(value),
		}, nil
	}
}

// DeleteItem removes an exact wire item ID.
func (c *Client) DeleteItem(ctx context.Context, itemID ItemID) (bool, error) {
	result, err := c.invokeRaw(ctx, SmithyOpcodeDelete, itemID, nil, SetOptions{})
	if err != nil {
		return false, operationError("delete item", err)
	}
	return deleteResult("delete item", result)
}

// Stats returns the server's JSON statistics document unchanged.
func (c *Client) Stats(ctx context.Context) (string, error) {
	result, err := c.invoke(ctx, SmithyOpcodeStats, nil, nil, SetOptions{})
	if err != nil {
		return "", operationError("stats", err)
	}
	if result.kind != SmithyFFIResultValue {
		return "", unexpectedResult("stats", result.kind)
	}
	return string(result.data), nil
}

// Sync waits for the server durability barrier.
func (c *Client) Sync(ctx context.Context) error {
	result, err := c.invoke(ctx, SmithyOpcodeSync, nil, nil, SetOptions{})
	if err != nil {
		return operationError("sync", err)
	}
	if result.kind != SmithyFFIResultOK {
		return unexpectedResult("sync", result.kind)
	}
	return nil
}

// Reconnect explicitly replaces the native connection without replaying an
// operation.
func (c *Client) Reconnect(ctx context.Context) error {
	result, err := c.invoke(ctx, SmithyFFIOperationReconnect, nil, nil, SetOptions{})
	if err != nil {
		return operationError("reconnect", err)
	}
	if result.kind != SmithyFFIResultOK {
		return unexpectedResult("reconnect", result.kind)
	}
	return nil
}

// ConnectionState returns a best-effort snapshot of the native connection.
func (c *Client) ConnectionState() ConnectionState {
	if c == nil {
		return ConnectionStateUnknown
	}
	c.mu.RLock()
	native := c.native
	if native == nil {
		c.mu.RUnlock()
		return ConnectionStateClosed
	}
	state := ConnectionState(native.state())
	c.mu.RUnlock()
	switch state {
	case ConnectionStateConnected,
		ConnectionStateReconnecting,
		ConnectionStateDisconnected,
		ConnectionStateClosed,
		ConnectionStateUnknown:
		return state
	default:
		return ConnectionStateUnknown
	}
}

// Close permanently closes the client. Repeated calls are safe.
func (c *Client) Close() error {
	if c == nil {
		return nil
	}
	c.mu.Lock()
	native := c.native
	if native == nil {
		c.mu.Unlock()
		return nil
	}
	c.native = nil
	err := native.close()
	c.mu.Unlock()
	runtime.SetFinalizer(c, nil)
	return err
}

func operationError(operation string, err error) error {
	if err == nil {
		return nil
	}
	if errors.Is(err, ErrClosed) {
		return err
	}
	var nativeErr *Error
	if errors.As(err, &nativeErr) {
		if nativeErr.Operation == "" {
			nativeErr.Operation = operation
		}
		return nativeErr
	}
	return &Error{Operation: operation, Message: err.Error(), Cause: err}
}

func unexpectedResult(operation string, kind uint32) error {
	return &Error{
		Operation: operation,
		Message:   fmt.Sprintf("native ABI returned unexpected result kind %d", kind),
	}
}

func getResult(operation string, result nativeResult) ([]byte, bool, error) {
	switch result.kind {
	case SmithyFFIResultValue:
		return result.data, true, nil
	case SmithyFFIResultNotFound:
		return nil, false, nil
	default:
		return nil, false, unexpectedResult(operation, result.kind)
	}
}

func setResult(operation string, result nativeResult) (SetOutcome, error) {
	switch result.kind {
	case SmithyFFIResultCreated:
		return Created, nil
	case SmithyFFIResultReplaced:
		return Replaced, nil
	case SmithyFFIResultNotStored:
		return NotStored, nil
	default:
		return "", unexpectedResult(operation, result.kind)
	}
}

func deleteResult(operation string, result nativeResult) (bool, error) {
	switch result.kind {
	case SmithyFFIResultDeleted:
		return true, nil
	case SmithyFFIResultNotDeleted:
		return false, nil
	default:
		return false, unexpectedResult(operation, result.kind)
	}
}
