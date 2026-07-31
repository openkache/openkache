package openkache

import "context"

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
	value, found, err := s.client.GetItem(ctx, itemID)
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
	options := SetOptions{}
	if input.Condition != nil {
		switch *input.Condition {
		case SmithySetConditionIfAbsent:
			options.Condition = IfAbsent
		case SmithySetConditionIfPresent:
			options.Condition = IfPresent
		default:
			return SmithySetOutput{}, validationError("set.condition", "unknown Smithy value")
		}
	}
	if input.TTLMilliseconds != nil {
		if *input.TTLMilliseconds < int64(SmithyClientMinimumPositiveValue) {
			return SmithySetOutput{}, validationError(
				"set.ttl_milliseconds",
				"must be greater than zero",
			)
		}
		options.TTLMillis = uint64(*input.TTLMilliseconds)
	}
	outcome, err := s.client.SetItem(ctx, itemID, input.Value, options)
	return SmithySetOutput{Outcome: outcome}, err
}

func (s smithyClient) Delete(
	ctx context.Context,
	input SmithyDeleteInput,
) (SmithyDeleteOutput, error) {
	itemID, err := NewItemID(input.ItemID)
	if err != nil {
		return SmithyDeleteOutput{}, err
	}
	deleted, err := s.client.DeleteItem(ctx, itemID)
	return SmithyDeleteOutput{Deleted: deleted}, err
}

func (s smithyClient) Stats(
	ctx context.Context,
	_ SmithyStatsInput,
) (SmithyStatsOutput, error) {
	json, err := s.client.Stats(ctx)
	return SmithyStatsOutput{JSON: json}, err
}

func (s smithyClient) Sync(
	ctx context.Context,
	_ SmithySyncInput,
) (SmithySyncOutput, error) {
	return SmithySyncOutput{}, s.client.Sync(ctx)
}
