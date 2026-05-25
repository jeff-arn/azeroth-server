# Testing rules

Honest acknowledgement: this project does not have a unit test framework wired up for the C++ game server. There is no `make test` target. The Python services have no `pytest` runner either. **Tested** here means specific things depending on the layer.

## What "tested" means by layer

### C++ game server (vmangos)

For now, "tested" means **a manual in-game smoke test** appropriate to the change. A change to combat AI is tested by running a dungeon with the affected role. A change to a packet handler is tested by performing the client action that produces that packet.

What to verify before pushing a non-trivial gameplay change:

- **Server builds clean.** No new compiler warnings against the current baseline.
- **Server boots without errors.** `Server.log` shows the usual startup banner without unexpected `ERROR` lines from the area you touched.
- **The feature does what you intended.** Log in. Trigger the path. Watch the bot, the spell, the packet, whatever you changed.
- **You didn't break something nearby.** If you touched a bot behavior, run a 5-man Deadmines clear (or equivalent) and watch for:
  - Tank survives normal pulls
  - Healer mana doesn't spiral
  - Casters stay at range and don't melee-walk
  - No movement spasms (formation stacking, jitter)
  - Pulls complete cleanly (no bots running in circles)

The PR description for any non-trivial gameplay change should note what was tested and what was observed. Server.log excerpts and screenshots count as evidence.

### Database changes

- **Apply locally first.** Run the new migration against a local copy of the relevant DB. Don't ship a migration you haven't run.
- **Re-run it.** Idempotent migrations should be a no-op the second time. If yours errors on second run, it's not idempotent — fix it.
- **Roll forward, not back.** Production phase doesn't have a "rollback the migration" path. If the migration breaks something, write a new migration that fixes the problem.

### Dashboard API

- **Hit the endpoint manually.** `curl http://localhost:5000/api/...` with realistic params.
- **Check response shape.** Compare against the previous response for an additive change — fields preserved, types unchanged.
- **Status codes match the situation.** 200 only when it actually worked. 4xx for client errors, 5xx for server bugs.

### Discord bot

- **Run it locally against a test channel.** Don't push to a Discord bot change without seeing it work in chat first. A test bot account + a private channel is the right setup.
- **Trigger the path.** Send the message that should produce a response. Verify the response.
- **Watch the logs.** `journalctl -u azeroth-discord-bot -f` (or stdout locally) — no unhandled exceptions.

## Adding real tests (when it makes sense)

Some code is inherently testable without a running server, and worth covering. The candidates:

- `scripts/generate_talent_builds.py` — pure data transformation. Could read a fixture DBC, write builds, snapshot the output.
- Time-of-day cosine curve calculations in the bot systems (crowd presence, guild presence). Pure math, deterministic given the time input.
- Dashboard input validation / query construction — Flask routes can be tested with the Flask test client.
- Discord bot command parsing — testable with mocked discord.py contexts.

If you add tests:

- C++: pick a single-header framework (Catch2 or doctest) if and when you commit to it. Put tests under `vmangos/tests/`. Wire a `make test` target. Don't add a test framework as a side effect of an unrelated change — it's its own project.
- Python: `pytest`. Tests under `dashboard/tests/` or `discord-bot/tests/`. Add a CI step to run them.

## What not to test

- **Generated code.** `BotTalentBuild` arrays come from `scripts/generate_talent_builds.py` reading DBC files. Test the generator, not its output.
- **Trivial getters and setters.** No payoff.
- **Third-party library behavior.** Trust that `discord.py` parses commands correctly; don't test discord.py from this repo.
- **The C++ standard library.** `std::vector` works.
- **Server uptime.** That's a monitoring concern, not a unit test concern.

## Naming conventions (when tests exist)

- C++ (Catch2/doctest): `TEST_CASE("description of expected behavior", "[tag]")`. Tag with the subsystem (`[bots]`, `[formation]`, `[ahbot]`).
- Python: `def test_<thing>_<condition>():`. E.g. `def test_cosine_curve_peaks_at_8pm():`.

## Test data

- Use small, representative fixtures committed to the repo. A test that loads 50MB of DBC files is too slow.
- Don't seed test data into the live DB. Tests that need DB context should use a separate test DB (containerized or `_test` suffix) that's wiped between runs.

## Failing tests

When (eventually) a test fails:

- **Investigate before rewriting.** A test that's been passing and now fails is either telling you about a real bug or about a behavior you intentionally changed. Either way, understand which before touching the test.
- **Fix the production code or fix the test — with intent.** Never delete a test "to make CI green" without a written reason.
- **A flaky test is a real test failure.** "It usually passes" means there's a race, an order dependency, or an external resource the test shouldn't depend on.

## Logs, screenshots, and the PR description

Until the test framework exists, the PR description is part of the test record. Include:

- What you changed (one sentence)
- What you ran to verify (Deadmines clear / curl call / Discord message)
- What you observed (success criteria met / interesting log excerpts)
- What you couldn't verify (any path the smoke test didn't cover)

A change without this section in the PR description is unfinished.
