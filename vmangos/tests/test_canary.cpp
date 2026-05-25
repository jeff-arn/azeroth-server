// Sanity check that the doctest toolchain is wired up.
#include "doctest.h"

TEST_CASE("canary: arithmetic")
{
    CHECK(2 + 2 == 4);
}

TEST_CASE("canary: floating point")
{
    CHECK(0.1 + 0.2 == doctest::Approx(0.3));
}
