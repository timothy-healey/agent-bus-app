-- 015 — LF34: recalibrate the window budget for the all-tokens basis (input +
-- output + cache_creation + cache_read). cache_read dominates real throughput,
-- so the old input+output-only budget (2_600_000) read far too low (~13% vs
-- claude.ai's ~35%). New default 190_000_000 derived from live throughput
-- (~67.2M ≈ 35% ⟹ ~192M, rounded). Tunable estimate, NOT an exact claude.ai
-- mirror (G6). Only bump rows still on the OLD default — never clobber a
-- budget the operator tuned in Settings.
UPDATE usage_config SET window_budget = 190000000
  WHERE id = 1 AND window_budget = 2600000;
