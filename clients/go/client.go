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
	// the server connection.
	Certificate []byte
	// Identity is optional mutual-TLS client authentication material.
	Identity *Identity
	// DataProtectionKey is the persistent 32-byte application secret.
	DataProtectionKey []byte
	// Compression is applied before the core's authenticated encryption.
	Compression CompressionOptions
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
	timeouts            TimeoutOptions
	retryAttempts       int
	maxInFlight         int
	nativeLibrary       string
}

func (o Options) normalize() (normalizedOptions, error) {
	if o.Address == "" {
		return normalizedOptions{}, validationError("address", "must not be empty")
	}
	if len(o.Certificate) == 0 {
		return normalizedOptions{}, validationError("certificate", "must not be empty")
	}
	if len(o.DataProtectionKey) != SmithyDataProtectionKeyBytes {
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
		connectTimeout = time.Duration(SmithyClientDefaultConnectTimeoutMS) * time.Millisecond
	}
	requestTimeout := o.Timeouts.Request
	if requestTimeout == 0 {
		requestTimeout = time.Duration(SmithyClientDefaultRequestTimeoutMS) * time.Millisecond
	}
	if connectTimeout < 0 || requestTimeout < 0 {
		return normalizedOptions{}, validationError("timeouts", "must not be negative")
	}
	if connectTimeout < time.Millisecond || requestTimeout < time.Millisecond {
		return normalizedOptions{}, validationError("timeouts", "must be at least one millisecond")
	}

	compression := o.Compression
	if compression.MinimumInputSize < 0 || compression.MinimumSavings < 0 {
		return normalizedOptions{}, validationError("compression", "size thresholds must not be negative")
	}
	if compression.Enabled {
		if compression.Level == 0 {
			compression.Level = SmithyClientDefaultCompressionLevel
		}
		if compression.Level < SmithyClientCompressionLevelMin ||
			compression.Level > SmithyClientCompressionLevelMax {
			return normalizedOptions{}, validationError(
				"compression.level",
				fmt.Sprintf(
					"must be from %d through %d",
					SmithyClientCompressionLevelMin,
					SmithyClientCompressionLevelMax,
				),
			)
		}
		if compression.MinimumInputSize == 0 {
			compression.MinimumInputSize = SmithyClientDefaultCompressionMinimumInputSize
		}
		if compression.MinimumSavings == 0 {
			compression.MinimumSavings = SmithyClientDefaultCompressionMinimumSavings
		}
	}

	retryAttempts := o.Retry.MaxAttempts
	if retryAttempts == 0 {
		retryAttempts = SmithyClientDefaultRetryMaxAttempts
	}
	if retryAttempts < 1 {
		return normalizedOptions{}, validationError("retry.max_attempts", "must be greater than zero")
	}
	maxInFlight := o.MaxInFlight
	if maxInFlight == 0 {
		maxInFlight = SmithyClientDefaultMaxInFlight
	}
	if maxInFlight < 1 {
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
			if bytes.HasPrefix(trimmed, []byte("-----BEGIN")) {
				identityCertificate = append(identityCertificate, trimmed...)
				identityCertificate = append(identityCertificate, '\n')
			} else if len(o.Identity.CertificateChain) == 1 {
				identityCertificate = append(identityCertificate, certificate...)
			} else {
				identityCertificate = append(
					identityCertificate,
					pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: certificate})...,
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
		timeouts:            TimeoutOptions{Connect: connectTimeout, Request: requestTimeout},
		retryAttempts:       retryAttempts,
		maxInFlight:         maxInFlight,
		nativeLibrary:       o.NativeLibrary,
	}, nil
}

func normalizedPEM(value []byte) []byte {
	trimmed := bytes.TrimSpace(value)
	if bytes.HasPrefix(trimmed, []byte("-----BEGIN")) {
		return append([]byte(nil), trimmed...)
	}
	return append([]byte(nil), value...)
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
	Condition SetCondition
	TTLMillis uint64
}

// ItemID is the exact fixed-width identifier carried by the wire protocol.
type ItemID [SmithyItemIDBytes]byte

// NewItemID copies an exact 32-byte wire item ID.
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

type nativeClient interface {
	execute(context.Context, uint32, []byte, []byte, SetOptions) (nativeResult, error)
	executeRaw(context.Context, uint32, ItemID, []byte, SetOptions) (nativeResult, error)
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
	if ctx == nil {
		return nativeResult{}, validationError("context", "must not be nil")
	}
	if err := ctx.Err(); err != nil {
		return nativeResult{}, err
	}
	c.mu.RLock()
	native := c.native
	if native == nil {
		c.mu.RUnlock()
		return nativeResult{}, ErrClosed
	}
	result, err := native.execute(ctx, operation, key, value, options)
	c.mu.RUnlock()
	return result, err
}

func (c *Client) invokeRaw(
	ctx context.Context,
	operation uint32,
	itemID ItemID,
	value []byte,
	options SetOptions,
) (nativeResult, error) {
	if ctx == nil {
		return nativeResult{}, validationError("context", "must not be nil")
	}
	if err := ctx.Err(); err != nil {
		return nativeResult{}, err
	}
	c.mu.RLock()
	native := c.native
	if native == nil {
		c.mu.RUnlock()
		return nativeResult{}, ErrClosed
	}
	result, err := native.executeRaw(ctx, operation, itemID, value, options)
	c.mu.RUnlock()
	return result, err
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
	if len(key) == 0 {
		return nil, false, validationError("key", "must not be empty")
	}
	result, err := c.invoke(ctx, SmithyOpcodeGet, key, nil, SetOptions{})
	if err != nil {
		return nil, false, operationError("get", err)
	}
	switch result.kind {
	case SmithyFFIResultValue:
		return result.data, true, nil
	case SmithyFFIResultNotFound:
		return nil, false, nil
	default:
		return nil, false, unexpectedResult("get", result.kind)
	}
}

// GetItem retrieves an exact wire item ID without application-key derivation or
// value protection.
func (c *Client) GetItem(ctx context.Context, itemID ItemID) ([]byte, bool, error) {
	result, err := c.invokeRaw(ctx, SmithyOpcodeGet, itemID, nil, SetOptions{})
	if err != nil {
		return nil, false, operationError("get item", err)
	}
	switch result.kind {
	case SmithyFFIResultValue:
		return result.data, true, nil
	case SmithyFFIResultNotFound:
		return nil, false, nil
	default:
		return nil, false, unexpectedResult("get item", result.kind)
	}
}

// Set encrypts and stores value for key.
func (c *Client) Set(ctx context.Context, key, value []byte, options SetOptions) (SetOutcome, error) {
	if len(key) == 0 {
		return "", validationError("key", "must not be empty")
	}
	if len(value) > SmithyMaxValueBytes {
		return "", validationError("value", fmt.Sprintf("exceeds %d bytes", SmithyMaxValueBytes))
	}
	if err := validateSetOptions(options); err != nil {
		return "", err
	}
	result, err := c.invoke(ctx, SmithyOpcodeSet, key, value, options)
	if err != nil {
		return "", operationError("set", err)
	}
	switch result.kind {
	case SmithyFFIResultCreated:
		return Created, nil
	case SmithyFFIResultReplaced:
		return Replaced, nil
	case SmithyFFIResultNotStored:
		return NotStored, nil
	default:
		return "", unexpectedResult("set", result.kind)
	}
}

func validateSetOptions(options SetOptions) error {
	if options.Condition != "" && options.Condition != IfAbsent && options.Condition != IfPresent {
		return validationError("set.condition", "must be empty, IfAbsent, or IfPresent")
	}
	return nil
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
	switch result.kind {
	case SmithyFFIResultCreated:
		return Created, nil
	case SmithyFFIResultReplaced:
		return Replaced, nil
	case SmithyFFIResultNotStored:
		return NotStored, nil
	default:
		return "", unexpectedResult("set item", result.kind)
	}
}

// Delete removes key and reports whether an item existed.
func (c *Client) Delete(ctx context.Context, key []byte) (bool, error) {
	if len(key) == 0 {
		return false, validationError("key", "must not be empty")
	}
	result, err := c.invoke(ctx, SmithyOpcodeDelete, key, nil, SetOptions{})
	if err != nil {
		return false, operationError("delete", err)
	}
	switch result.kind {
	case SmithyFFIResultDeleted:
		return true, nil
	case SmithyFFIResultNotDeleted:
		return false, nil
	default:
		return false, unexpectedResult("delete", result.kind)
	}
}

// DeleteItem removes an exact wire item ID.
func (c *Client) DeleteItem(ctx context.Context, itemID ItemID) (bool, error) {
	result, err := c.invokeRaw(ctx, SmithyOpcodeDelete, itemID, nil, SetOptions{})
	if err != nil {
		return false, operationError("delete item", err)
	}
	switch result.kind {
	case SmithyFFIResultDeleted:
		return true, nil
	case SmithyFFIResultNotDeleted:
		return false, nil
	default:
		return false, unexpectedResult("delete item", result.kind)
	}
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
