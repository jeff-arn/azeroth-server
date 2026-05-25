// Tests for BreadcrumbTrail (ring-buffer of leader path waypoints).
// Pure C++14 ring-buffer logic — no game state required.
#include "doctest.h"

#include "BotFormation.h"

TEST_CASE("BreadcrumbTrail: empty trail reports zero size")
{
    BreadcrumbTrail t;
    CHECK(t.Size() == 0);
    CHECK(t.IsEmpty());

    BreadcrumbPoint out;
    CHECK_FALSE(t.GetPoint(0, out));
}

TEST_CASE("BreadcrumbTrail: first point is recorded")
{
    BreadcrumbTrail t;
    t.RecordPoint(100.0f, 200.0f, 50.0f, 0.0f, 1000);
    CHECK(t.Size() == 1);
    CHECK_FALSE(t.IsEmpty());

    BreadcrumbPoint p;
    REQUIRE(t.GetPoint(0, p));
    CHECK(p.x == doctest::Approx(100.0f));
    CHECK(p.y == doctest::Approx(200.0f));
    CHECK(p.z == doctest::Approx(50.0f));
}

TEST_CASE("BreadcrumbTrail: throttles by record interval")
{
    BreadcrumbTrail t;
    t.RecordPoint(0.0f, 0.0f, 0.0f, 0.0f, 1000);
    // Same tick — must not record (interval is 1000ms).
    t.RecordPoint(10.0f, 10.0f, 10.0f, 0.0f, 1500);
    CHECK(t.Size() == 1);
    // One interval later, far enough away — records.
    t.RecordPoint(10.0f, 10.0f, 10.0f, 0.0f, 2000);
    CHECK(t.Size() == 2);
}

TEST_CASE("BreadcrumbTrail: throttles by min-distance squared (2yd)")
{
    BreadcrumbTrail t;
    t.RecordPoint(0.0f, 0.0f, 0.0f, 0.0f, 1000);
    // 1yd away — under the 2yd threshold; interval already elapsed.
    t.RecordPoint(1.0f, 0.0f, 0.0f, 0.0f, 2500);
    CHECK(t.Size() == 1);
    // 3yd away — over threshold.
    t.RecordPoint(3.0f, 0.0f, 0.0f, 0.0f, 4000);
    CHECK(t.Size() == 2);
}

TEST_CASE("BreadcrumbTrail: Clear() empties the buffer")
{
    BreadcrumbTrail t;
    t.RecordPoint(1.0f, 1.0f, 1.0f, 0.0f, 1000);
    t.RecordPoint(5.0f, 5.0f, 5.0f, 0.0f, 2000);
    REQUIRE(t.Size() == 2);
    t.Clear();
    CHECK(t.Size() == 0);
    CHECK(t.IsEmpty());
}

TEST_CASE("BreadcrumbTrail: ring buffer wraps after MAX_CRUMBS")
{
    BreadcrumbTrail t;
    constexpr uint32 N = BreadcrumbTrail::MAX_CRUMBS;
    // Record N+5 points well-separated in time and space.
    for (uint32 i = 0; i < N + 5; ++i)
    {
        t.RecordPoint(float(i) * 10.0f, 0.0f, 0.0f, 0.0f, 1000 + i * 1000);
    }
    // Size is capped at MAX_CRUMBS.
    CHECK(t.Size() == N);
    // Newest point (offset 0) is the last one written.
    BreadcrumbPoint newest;
    REQUIRE(t.GetPoint(0, newest));
    CHECK(newest.x == doctest::Approx(float(N + 4) * 10.0f));
}

TEST_CASE("BreadcrumbTrail: GetPoint reads newest-to-oldest by offset")
{
    BreadcrumbTrail t;
    t.RecordPoint(10.0f, 0.0f, 0.0f, 0.0f, 1000);
    t.RecordPoint(20.0f, 0.0f, 0.0f, 0.0f, 2000);
    t.RecordPoint(30.0f, 0.0f, 0.0f, 0.0f, 3000);

    BreadcrumbPoint p;
    REQUIRE(t.GetPoint(0, p));
    CHECK(p.x == doctest::Approx(30.0f));  // newest
    REQUIRE(t.GetPoint(1, p));
    CHECK(p.x == doctest::Approx(20.0f));
    REQUIRE(t.GetPoint(2, p));
    CHECK(p.x == doctest::Approx(10.0f));  // oldest
    CHECK_FALSE(t.GetPoint(3, p));         // out of range
}

TEST_CASE("BreadcrumbTrail: GetPointAtDistance walks arc length backward")
{
    // Points laid out in a straight line along x: 0, 5, 10, 15, 20.
    BreadcrumbTrail t;
    for (int i = 0; i < 5; ++i)
        t.RecordPoint(float(i) * 5.0f, 0.0f, 0.0f, 0.0f, 1000 + i * 2000);

    BreadcrumbPoint out;
    // 0yd back from head = the newest point itself (x=20).
    REQUIRE(t.GetPointAtDistance(0.0f, out));
    CHECK(out.x == doctest::Approx(20.0f));

    // 5yd back along the trail = previous point (x=15).
    REQUIRE(t.GetPointAtDistance(5.0f, out));
    CHECK(out.x == doctest::Approx(15.0f));

    // 7yd back = between 15 and 10 — interpolated.
    REQUIRE(t.GetPointAtDistance(7.0f, out));
    CHECK(out.x == doctest::Approx(13.0f));  // 15 - (2/5)*5 = 13
}

TEST_CASE("BreadcrumbTrail: FindNearestIndex picks closest point")
{
    BreadcrumbTrail t;
    t.RecordPoint(0.0f, 0.0f, 0.0f, 0.0f, 1000);
    t.RecordPoint(10.0f, 0.0f, 0.0f, 0.0f, 2000);
    t.RecordPoint(20.0f, 0.0f, 0.0f, 0.0f, 3000);

    // Closest to (11, 0, 0) is the offset-1 point (x=10).
    CHECK(t.FindNearestIndex(11.0f, 0.0f, 0.0f) == 1);
    // Closest to (-1, 0, 0) is the offset-2 point (x=0, oldest).
    CHECK(t.FindNearestIndex(-1.0f, 0.0f, 0.0f) == 2);
    // Closest to (19, 0, 0) is the offset-0 point (x=20, newest).
    CHECK(t.FindNearestIndex(19.0f, 0.0f, 0.0f) == 0);
}
