// Tests for IsChatCooldownExpired — the free function extracted from
// BotChatManager::CanChat so cooldown math is testable without
// time(nullptr).  The plan called this out as the "time_t fireTime"
// gotcha class: countdown-style timers collapse multi-message chains
// on the 60s WUPDATE_AUCTIONS tick.
#include "doctest.h"

#include "BotChatManager.h"

TEST_CASE("IsChatCooldownExpired: never-chatted bot is always allowed")
{
    CHECK(IsChatCooldownExpired(1000, 0, 60) == true);
    CHECK(IsChatCooldownExpired(0, 0, 60) == true);
}

TEST_CASE("IsChatCooldownExpired: within cooldown window")
{
    // lastChat = 1000, now = 1059, cooldown = 60 — 59s elapsed, still blocked.
    CHECK(IsChatCooldownExpired(1059, 1000, 60) == false);
}

TEST_CASE("IsChatCooldownExpired: exact boundary")
{
    // 60s elapsed at cooldown=60 — allowed (>=).
    CHECK(IsChatCooldownExpired(1060, 1000, 60) == true);
}

TEST_CASE("IsChatCooldownExpired: past cooldown")
{
    CHECK(IsChatCooldownExpired(2000, 1000, 60) == true);
    CHECK(IsChatCooldownExpired(5000, 1000, 30) == true);
}

TEST_CASE("IsChatCooldownExpired: clock skew (now < lastChat) blocks")
{
    // If now goes backward (NTP step, container migration), we MUST
    // NOT flood with chat. Return false.
    CHECK(IsChatCooldownExpired(500, 1000, 60) == false);
}

TEST_CASE("IsChatCooldownExpired: zero cooldown always passes")
{
    // Sentinel: cooldown==0 means "no rate limit" — second message
    // immediately after the first is fine.
    CHECK(IsChatCooldownExpired(1001, 1000, 0) == true);
    CHECK(IsChatCooldownExpired(1000, 1000, 0) == true);
}
