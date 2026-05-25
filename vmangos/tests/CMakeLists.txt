# vmangos_tests — doctest-based unit tests for pure C++ logic.
#
# Links the full `game` static library because the bot AI sources are
# entangled with Player/Map/World headers; refactoring for interface
# isolation is a separate project. First build is slow, incremental
# rebuilds of just the test code are fast.

set(TEST_SOURCES
    test_main.cpp
    test_stubs.cpp
    test_canary.cpp
    test_breadcrumb_trail.cpp
    test_behavior_scores.cpp
    test_chat_cooldown.cpp
)

add_executable(vmangos_tests ${TEST_SOURCES})

target_include_directories(vmangos_tests PRIVATE
    ${CMAKE_SOURCE_DIR}/dep/doctest
    ${CMAKE_SOURCE_DIR}/src/game
    ${CMAKE_SOURCE_DIR}/src/game/PlayerBots
    ${CMAKE_SOURCE_DIR}/src/shared
    ${CMAKE_SOURCE_DIR}/src/framework
    ${CMAKE_SOURCE_DIR}/dep/include
)

target_link_libraries(vmangos_tests PRIVATE
    game
    scripts
    shared
    framework
    g3dlite
    gsoap
)

if (UNIX)
    target_link_libraries(vmangos_tests PRIVATE
        ${MYSQL_LIBRARY}
        ${OPENSSL_LIBRARIES}
        ${OPENSSL_EXTRA_LIBRARIES}
        ${ZLIB_LIBRARIES}
    )
endif()

add_test(NAME vmangos_tests COMMAND vmangos_tests)
