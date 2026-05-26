// Tests for the pure Score() functions on BotBehavior subclasses.
//
// The plan called out a v0.8→v0.9 regression class: behaviors that
// return Score > 0 but always fail in Execute(), blocking lower-scoring
// behaviors from running. These tests pin the Score() math against
// known SituationContext snapshots.
#include "doctest.h"

#include "BotBehaviors_Shared.h"
#include "BotSituationContext.h"
#include "SharedDefines.h"

TEST_CASE("BehaviorSelfPreserve: silent above 20% HP")
{
    SituationContext ctx;
    BehaviorSelfPreserve b(nullptr);  // Score() doesn't dereference m_ai

    ctx.healthPct = 100.0f;
    CHECK(b.Score(ctx) == doctest::Approx(0.0f));
    ctx.healthPct = 25.0f;
    CHECK(b.Score(ctx) == doctest::Approx(0.0f));
    ctx.healthPct = 20.1f;
    CHECK(b.Score(ctx) == doctest::Approx(0.0f));
}

TEST_CASE("BehaviorSelfPreserve: fires below 20% HP")
{
    SituationContext ctx;
    BehaviorSelfPreserve b(nullptr);

    // At 20% HP exactly: 90 + (20-20)*0.25 = 90
    ctx.healthPct = 20.0f;
    CHECK(b.Score(ctx) == doctest::Approx(90.0f));
    // At 10% HP: 90 + 10*0.25 = 92.5
    ctx.healthPct = 10.0f;
    CHECK(b.Score(ctx) == doctest::Approx(92.5f));
    // At 0% HP: 90 + 20*0.25 = 95
    ctx.healthPct = 0.0f;
    CHECK(b.Score(ctx) == doctest::Approx(95.0f));
}

TEST_CASE("BehaviorSelfPreserve: monotonic — lower HP scores higher")
{
    SituationContext ctx;
    BehaviorSelfPreserve b(nullptr);

    ctx.healthPct = 15.0f;
    float at15 = b.Score(ctx);
    ctx.healthPct = 5.0f;
    float at5 = b.Score(ctx);
    CHECK(at5 > at15);
}

TEST_CASE("BehaviorKite: silent when not being melee'd")
{
    SituationContext ctx;
    BehaviorKite b(nullptr);

    ctx.isBeingMeleed = false;
    ctx.role = ROLE_RANGE_DPS;
    ctx.healthPct = 50.0f;
    CHECK(b.Score(ctx) == doctest::Approx(0.0f));
}

TEST_CASE("BehaviorKite: silent for melee role even when melee'd")
{
    SituationContext ctx;
    BehaviorKite b(nullptr);

    ctx.isBeingMeleed = true;
    ctx.role = ROLE_MELEE_DPS;
    ctx.healthPct = 50.0f;
    CHECK(b.Score(ctx) == doctest::Approx(0.0f));
}

TEST_CASE("BehaviorKite: fires for caster being melee'd, scales with damage")
{
    SituationContext ctx;
    BehaviorKite b(nullptr);

    ctx.isBeingMeleed = true;
    ctx.role = ROLE_RANGE_DPS;
    ctx.playerClass = 8;  // Mage

    // At 100% HP: 30 + 0*0.57 = 30
    ctx.healthPct = 100.0f;
    CHECK(b.Score(ctx) == doctest::Approx(30.0f));
    // At 30% HP: 30 + 70*0.57 = 69.9
    ctx.healthPct = 30.0f;
    CHECK(b.Score(ctx) == doctest::Approx(69.9f));
    // At 0% HP: 30 + 100*0.57 = 87
    ctx.healthPct = 0.0f;
    CHECK(b.Score(ctx) == doctest::Approx(87.0f));
}

TEST_CASE("BehaviorKite: healer with dying tank does NOT kite if healer is healthy")
{
    // Regression guard for the documented gotcha: healers must stand and
    // heal a dying tank instead of running away.
    SituationContext ctx;
    BehaviorKite b(nullptr);

    ctx.isBeingMeleed = true;
    ctx.role = ROLE_HEALER;
    ctx.tankIsAlive = true;
    ctx.tankHealthPct = 20.0f;   // tank dying
    ctx.healthPct = 80.0f;        // healer fine
    CHECK(b.Score(ctx) == doctest::Approx(0.0f));
}

TEST_CASE("BehaviorKite: healer with dying tank STILL kites if healer is also dying")
{
    SituationContext ctx;
    BehaviorKite b(nullptr);

    ctx.isBeingMeleed = true;
    ctx.role = ROLE_HEALER;
    ctx.tankIsAlive = true;
    ctx.tankHealthPct = 20.0f;
    ctx.healthPct = 40.0f;        // healer also in trouble
    CHECK(b.Score(ctx) > 0.0f);
}
