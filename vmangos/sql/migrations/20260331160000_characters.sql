DROP PROCEDURE IF EXISTS add_migration;
DELIMITER ??
CREATE PROCEDURE `add_migration`()
BEGIN
DECLARE v INT DEFAULT 1;
SET v = (SELECT COUNT(*) FROM `migrations` WHERE `id`='20260331160000');
IF v = 0 THEN
INSERT INTO `migrations` VALUES ('20260331160000');
-- Add your query below.

-- ============================================================================
-- 15 bot guilds to make crowd bots feel like real players.
-- Hardcore (5): edgy names, level 60 city bots
-- Social (5): fun names, inn bots (mixed levels)
-- Mix (5): fantasy names, blend of city + inn bots
--
-- Guild IDs 10-24 (skip 2-9 for future player guilds)
-- Each crowd bot is in exactly one guild. ~10 members each.
-- ============================================================================

INSERT INTO `guild` (`guild_id`, `name`, `leader_guid`, `emblem_style`, `emblem_color`, `border_style`, `border_color`, `background_color`, `info`, `motd`, `create_date`) VALUES
(10, 'Oblivion',             100, 7, 0,  1, 0,  47, '', 'Farm night Wednesday. Bring consumes.', UNIX_TIMESTAMP()),
(11, 'Nihilum',              104, 3, 17, 0, 0,  25, '', 'World buffs before raid or sit.', UNIX_TIMESTAMP()),
(12, 'Retribution',          108, 5, 4,  3, 0,  41, '', 'DKP updated on forums.', UNIX_TIMESTAMP()),
(13, 'Decimation',           115, 1, 12, 2, 0,  34, '', 'Naxx signups are open.', UNIX_TIMESTAMP()),
(14, 'Exodus',               125, 6, 7,  4, 0,  50, '', 'Parse or bench.', UNIX_TIMESTAMP()),
(15, 'Casually Late',        300, 0, 3,  0, 3,  15, '', 'No drama, just vibes', UNIX_TIMESTAMP()),
(16, 'Loot And Scoot',       303, 2, 14, 1, 14, 22, '', 'Need before Greed... usually', UNIX_TIMESTAMP()),
(17, 'AFK In Ironforge',     321, 4, 6,  2, 6,  8,  '', 'Recruiting all levels! /w for invite', UNIX_TIMESTAMP()),
(18, 'The Zug Life',         348, 8, 10, 3, 10, 38, '', 'zug zug', UNIX_TIMESTAMP()),
(19, 'Pull The Whole Room',  342, 0, 1,  4, 1,  45, '', 'Leeroy would be proud', UNIX_TIMESTAMP()),
(20, 'Dawn of Azeroth',      135, 1, 5,  0, 5,  12, '', 'Dungeons tonight, whisper for invite', UNIX_TIMESTAMP()),
(21, 'Ironbound Legion',     158, 3, 0,  1, 0,  30, '', 'PvE focused, all welcome', UNIX_TIMESTAMP()),
(22, 'Silver Covenant',      145, 5, 15, 2, 15, 20, '', 'Raid and chill', UNIX_TIMESTAMP()),
(23, 'Frostmane Exiles',     160, 7, 11, 3, 11, 42, '', 'BRD runs daily', UNIX_TIMESTAMP()),
(24, 'The Scarlet Pariahs',  175, 2, 4,  4, 4,  36, '', 'Dungeon grind to 60 then raid', UNIX_TIMESTAMP());

-- Ranks for all 15 guilds
INSERT INTO `guild_rank` (`guild_id`, `id`, `name`, `rights`) VALUES
(10,0,'Guild Master',1044991),(10,1,'Officer',1044991),(10,2,'Raider',67),(10,3,'Trial',67),(10,4,'Recruit',67),
(11,0,'Guild Master',1044991),(11,1,'Officer',1044991),(11,2,'Raider',67),(11,3,'Trial',67),(11,4,'Recruit',67),
(12,0,'Guild Master',1044991),(12,1,'Officer',1044991),(12,2,'Raider',67),(12,3,'Trial',67),(12,4,'Recruit',67),
(13,0,'Guild Master',1044991),(13,1,'Officer',1044991),(13,2,'Raider',67),(13,3,'Trial',67),(13,4,'Recruit',67),
(14,0,'Guild Master',1044991),(14,1,'Officer',1044991),(14,2,'Raider',67),(14,3,'Trial',67),(14,4,'Recruit',67),
(15,0,'Guild Master',1044991),(15,1,'Officer',1044991),(15,2,'Member',67),(15,3,'Alt',67),(15,4,'Newbie',67),
(16,0,'Guild Master',1044991),(16,1,'Officer',1044991),(16,2,'Member',67),(16,3,'Alt',67),(16,4,'Newbie',67),
(17,0,'Guild Master',1044991),(17,1,'Officer',1044991),(17,2,'Member',67),(17,3,'Alt',67),(17,4,'Newbie',67),
(18,0,'Guild Master',1044991),(18,1,'Officer',1044991),(18,2,'Member',67),(18,3,'Alt',67),(18,4,'Newbie',67),
(19,0,'Guild Master',1044991),(19,1,'Officer',1044991),(19,2,'Member',67),(19,3,'Alt',67),(19,4,'Newbie',67),
(20,0,'Guild Master',1044991),(20,1,'Officer',1044991),(20,2,'Veteran',67),(20,3,'Member',67),(20,4,'Initiate',67),
(21,0,'Guild Master',1044991),(21,1,'Officer',1044991),(21,2,'Veteran',67),(21,3,'Member',67),(21,4,'Initiate',67),
(22,0,'Guild Master',1044991),(22,1,'Officer',1044991),(22,2,'Veteran',67),(22,3,'Member',67),(22,4,'Initiate',67),
(23,0,'Guild Master',1044991),(23,1,'Officer',1044991),(23,2,'Veteran',67),(23,3,'Member',67),(23,4,'Initiate',67),
(24,0,'Guild Master',1044991),(24,1,'Officer',1044991),(24,2,'Veteran',67),(24,3,'Member',67),(24,4,'Initiate',67);

-- ============================================================================
-- Guild membership — each bot in exactly one guild
-- ============================================================================

INSERT IGNORE INTO `guild_member` (`guild_id`, `guid`, `rank`, `player_note`, `officer_note`) VALUES
-- Oblivion (10) — IF bank front [100-103, 105-107, 109-111]
(10,100,0,'',''),(10,101,2,'',''),(10,102,2,'',''),(10,103,2,'',''),(10,105,2,'',''),
(10,106,3,'',''),(10,107,2,'',''),(10,109,2,'',''),(10,110,2,'',''),(10,111,3,'',''),

-- Nihilum (11) — IF bank front [104, 112-114, 116-120]
(11,104,0,'',''),(11,112,2,'',''),(11,113,2,'',''),(11,114,2,'',''),(11,116,2,'',''),
(11,117,3,'',''),(11,118,2,'',''),(11,119,2,'',''),(11,120,2,'',''),(11,121,3,'',''),

-- Retribution (12) — IF bank front [108, 122-124, 146-150]
(12,108,0,'',''),(12,122,2,'',''),(12,123,2,'',''),(12,124,2,'',''),(12,146,2,'',''),
(12,147,3,'',''),(12,148,2,'',''),(12,149,2,'',''),(12,150,2,'',''),(12,153,3,'',''),

-- Decimation (13) — IF bank front + bank interior [115, 151-157, 170-171]
(13,115,0,'',''),(13,151,2,'',''),(13,152,2,'',''),(13,154,2,'',''),(13,155,2,'',''),
(13,156,3,'',''),(13,157,2,'',''),(13,170,2,'',''),(13,171,3,'',''),(13,172,2,'',''),

-- Exodus (14) — SW AH + SW Bank [125-134]
(14,125,0,'',''),(14,126,2,'',''),(14,127,2,'',''),(14,128,2,'',''),(14,129,2,'',''),
(14,130,3,'',''),(14,131,2,'',''),(14,132,2,'',''),(14,133,2,'',''),(14,134,3,'',''),

-- Casually Late (15) — inn bots EK south [300-302, 306-308, 309-311]
(15,300,0,'',''),(15,301,3,'',''),(15,302,3,'',''),(15,306,3,'',''),(15,307,4,'',''),
(15,308,3,'',''),(15,309,3,'',''),(15,310,4,'',''),(15,311,3,'',''),(15,336,4,'',''),

-- Loot And Scoot (16) — inn bots EK mid [303-305, 315-320]
(16,303,0,'',''),(16,304,3,'',''),(16,305,3,'',''),(16,315,3,'',''),(16,316,4,'',''),
(16,317,3,'',''),(16,318,3,'',''),(16,319,4,'',''),(16,320,3,'',''),(16,337,4,'',''),

-- AFK In Ironforge (17) — inn bots EK north [321-329]
(17,321,0,'',''),(17,322,3,'',''),(17,323,3,'',''),(17,324,3,'',''),(17,325,4,'',''),
(17,326,3,'',''),(17,327,3,'',''),(17,328,4,'',''),(17,329,3,'',''),(17,338,4,'',''),

-- The Zug Life (18) — inn bots Kalimdor south [348-353, 360-362]
(18,348,0,'',''),(18,349,3,'',''),(18,350,3,'',''),(18,351,3,'',''),(18,352,4,'',''),
(18,353,3,'',''),(18,360,3,'',''),(18,361,4,'',''),(18,362,3,'',''),(18,369,4,'',''),

-- Pull The Whole Room (19) — inn bots Kalimdor north [342-347, 363-365]
(19,342,0,'',''),(19,343,3,'',''),(19,344,3,'',''),(19,345,3,'',''),(19,346,4,'',''),
(19,347,3,'',''),(19,363,3,'',''),(19,364,4,'',''),(19,365,3,'',''),(19,370,4,'',''),

-- Dawn of Azeroth (20) — SW bank + inn [135-139, 330-332, 339-341]
(20,135,0,'',''),(20,136,3,'',''),(20,137,3,'',''),(20,138,2,'',''),(20,139,3,'',''),
(20,330,3,'',''),(20,331,3,'',''),(20,332,4,'',''),(20,339,3,'',''),(20,340,4,'',''),

-- Ironbound Legion (21) — IF bank front + inn [158-159, 166-169, 333-335, 341]
(21,158,0,'',''),(21,159,3,'',''),(21,166,3,'',''),(21,167,2,'',''),(21,168,3,'',''),
(21,169,3,'',''),(21,333,3,'',''),(21,334,4,'',''),(21,335,3,'',''),(21,341,4,'',''),

-- Silver Covenant (22) — IF bank front + inn [145, 161-165, 354-356]
(22,145,0,'',''),(22,161,3,'',''),(22,162,3,'',''),(22,163,2,'',''),(22,164,3,'',''),
(22,165,3,'',''),(22,354,3,'',''),(22,355,4,'',''),(22,356,3,'',''),(22,371,4,'',''),

-- Frostmane Exiles (23) — IF bots + inn [160, 173-174, 357-359, 366-368]
(23,160,0,'',''),(23,173,3,'',''),(23,174,3,'',''),(23,357,2,'',''),(23,358,3,'',''),
(23,359,3,'',''),(23,366,3,'',''),(23,367,4,'',''),(23,368,3,'',''),(23,312,4,'',''),

-- The Scarlet Pariahs (24) — IF AH bots + Goldshire [175-179, 140-144]
(24,175,0,'',''),(24,176,3,'',''),(24,177,3,'',''),(24,178,2,'',''),(24,179,3,'',''),
(24,140,3,'',''),(24,141,4,'',''),(24,142,3,'',''),(24,143,4,'',''),(24,144,3,'','');

-- End of migration.
END IF;
END??
DELIMITER ;
CALL add_migration();
DROP PROCEDURE IF EXISTS add_migration;
