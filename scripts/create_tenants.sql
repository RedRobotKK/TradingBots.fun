-- TradingBots.fun tenant seed data
-- Human-named accounts with capital labels for public display.
-- Format: "Name ($Amount)" — shown on the public stats page and leaderboard.
-- Generated: 2026-03-18

INSERT INTO tenants (id, wallet_address, display_name, initial_capital)
VALUES (
  '3e58552b-5a45-4d11-b22d-4834c0ba98a7',
  '0x15ae283c5396095c8f01c47e0a7f8c236431cfa6',
  'Bob ($1M)',
  1000000.0
) ON CONFLICT (wallet_address) DO UPDATE SET display_name = EXCLUDED.display_name;

INSERT INTO tenants (id, wallet_address, display_name, initial_capital)
VALUES (
  'c9940170-5fe8-4c30-aab1-39ef334bc162',
  '0x4abfc01d1e0fbdc2305d70a252e87bd2a8e1d430',
  'Alice ($100K)',
  100000.0
) ON CONFLICT (wallet_address) DO UPDATE SET display_name = EXCLUDED.display_name;

INSERT INTO tenants (id, wallet_address, display_name, initial_capital)
VALUES (
  '8bef069b-28cd-497a-9030-de3312300fe5',
  '0xdc4cc408765c6dac8d241e38332ce12eedc6d074',
  'Steve ($10K)',
  10000.0
) ON CONFLICT (wallet_address) DO UPDATE SET display_name = EXCLUDED.display_name;

INSERT INTO tenants (id, wallet_address, display_name, initial_capital)
VALUES (
  'df368c08-5e4c-4d4c-b646-1be75211894e',
  '0xac1842ff30ab71105658c535dabc640b7ec503d9',
  'Stacey ($10K)',
  10000.0
) ON CONFLICT (wallet_address) DO UPDATE SET display_name = EXCLUDED.display_name;

INSERT INTO tenants (id, wallet_address, display_name, initial_capital)
VALUES (
  'f16eec18-a0c7-4a57-9361-ed63429fd927',
  '0x90aee3d82a5c6ea61532b9e0d7f6225e45242e37',
  'Linda ($1K)',
  1000.0
) ON CONFLICT (wallet_address) DO UPDATE SET display_name = EXCLUDED.display_name;

INSERT INTO tenants (id, wallet_address, display_name, initial_capital)
VALUES (
  '7eba6a49-9e81-4270-ae47-95a39d60eb5b',
  '0x548ebe83b46bf3844589b49871ba44319d7b0d61',
  'James ($1K)',
  1000.0
) ON CONFLICT (wallet_address) DO UPDATE SET display_name = EXCLUDED.display_name;

INSERT INTO tenants (id, wallet_address, display_name, initial_capital)
VALUES (
  '3fe50a3d-4768-4c55-8b0f-3be2cada8c1a',
  '0xa25eaa117c80c9779d689898c8b0588db906fbe5',
  'Bill ($100)',
  100.0
) ON CONFLICT (wallet_address) DO UPDATE SET display_name = EXCLUDED.display_name;

INSERT INTO tenants (id, wallet_address, display_name, initial_capital)
VALUES (
  '52565220-4072-4802-ab44-e6d075ad9894',
  '0xb01d05702a491271c9730c36d30965ed23206182',
  'Carol ($10)',
  10.0
) ON CONFLICT (wallet_address) DO UPDATE SET display_name = EXCLUDED.display_name;

INSERT INTO tenants (id, wallet_address, display_name, initial_capital)
VALUES (
  '1c9092bb-e97f-4d5c-be29-7ff218b6754c',
  '0x4fb773caf21c67ded2165af501a71d93e2665921',
  'Mike ($10)',
  10.0
) ON CONFLICT (wallet_address) DO UPDATE SET display_name = EXCLUDED.display_name;
