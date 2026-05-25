// Stubs for globals that mangosd's Main.cpp normally defines.
//
// libgame.a / libshared.a reference these symbols (CharacterDatabase,
// WorldDatabase, LoginDatabase, g_mainLogFileName) but the test binary
// doesn't include Main.cpp. Define them here as empty instances so the
// link resolves. Tests should not exercise code paths that actually
// hit the DB — those need the integration suite (phase 5).
#include "Database/DatabaseEnv.h"

DatabaseType WorldDatabase;
DatabaseType CharacterDatabase;
DatabaseType LoginDatabase;
DatabaseType LogsDatabase;

uint32 realmID = 0;
char const* g_mainLogFileName = "Test.log";
