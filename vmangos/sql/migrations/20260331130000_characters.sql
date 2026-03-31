DROP PROCEDURE IF EXISTS add_migration;
DELIMITER ??
CREATE PROCEDURE `add_migration`()
BEGIN
DECLARE v INT DEFAULT 1;
SET v = (SELECT COUNT(*) FROM `migrations` WHERE `id`='20260331130000');
IF v = 0 THEN
INSERT INTO `migrations` VALUES ('20260331130000');
-- Add your query below.

-- Additional 25 crowd bots for Ironforge Bank front (GUIDs 145-169, account 97)
-- Doubles IF crowd bot presence from 25 to 50
INSERT IGNORE INTO `characters` (`guid`, `account`, `name`, `race`, `class`, `gender`, `level`, `zone`, `map`, `position_x`, `position_y`, `position_z`, `orientation`, `money`)
VALUES
(145, 97, 'Anvilor',     3, 1, 0, 60, 1537, 0, -4920.0, -958.0, 501.5, 0, 0),
(146, 97, 'Velindra',    1, 5, 1, 60, 1537, 0, -4920.0, -958.0, 501.5, 0, 0),
(147, 97, 'Grundar',     3, 3, 0, 60, 1537, 0, -4920.0, -958.0, 501.5, 0, 0),
(148, 97, 'Elspeth',     1, 8, 1, 60, 1537, 0, -4920.0, -958.0, 501.5, 0, 0),
(149, 97, 'Thrainor',    3, 2, 0, 60, 1537, 0, -4920.0, -958.0, 501.5, 0, 0),
(150, 97, 'Moonveil',    4, 11, 1, 60, 1537, 0, -4920.0, -958.0, 501.5, 0, 0),
(151, 97, 'Cogburn',     7, 9, 0, 60, 1537, 0, -4920.0, -958.0, 501.5, 0, 0),
(152, 97, 'Aldara',      1, 4, 1, 60, 1537, 0, -4920.0, -958.0, 501.5, 0, 0),
(153, 97, 'Hammerfel',   3, 1, 0, 60, 1537, 0, -4920.0, -958.0, 501.5, 0, 0),
(154, 97, 'Silvaine',    4, 3, 1, 60, 1537, 0, -4920.0, -958.0, 501.5, 0, 0),
(155, 97, 'Boltwick',    7, 8, 0, 60, 1537, 0, -4920.0, -958.0, 501.5, 0, 0),
(156, 97, 'Margaux',     1, 9, 1, 60, 1537, 0, -4920.0, -958.0, 501.5, 0, 0),
(157, 97, 'Stonebraid',  3, 5, 0, 60, 1537, 0, -4920.0, -958.0, 501.5, 0, 0),
(158, 97, 'Thornby',     1, 1, 0, 60, 1537, 0, -4920.0, -958.0, 501.5, 0, 0),
(159, 97, 'Glimmera',    7, 4, 1, 60, 1537, 0, -4920.0, -958.0, 501.5, 0, 0),
(160, 97, 'Dawnhollow',  4, 5, 1, 60, 1537, 0, -4920.0, -958.0, 501.5, 0, 0),
(161, 97, 'Ferrik',      3, 3, 0, 60, 1537, 0, -4920.0, -958.0, 501.5, 0, 0),
(162, 97, 'Clarabelle',  1, 2, 1, 60, 1537, 0, -4920.0, -958.0, 501.5, 0, 0),
(163, 97, 'Gearjaw',     7, 1, 0, 60, 1537, 0, -4920.0, -958.0, 501.5, 0, 0),
(164, 97, 'Starfallow',  4, 4, 1, 60, 1537, 0, -4920.0, -958.0, 501.5, 0, 0),
(165, 97, 'Brogan',      3, 2, 0, 60, 1537, 0, -4920.0, -958.0, 501.5, 0, 0),
(166, 97, 'Penellis',    1, 8, 1, 60, 1537, 0, -4920.0, -958.0, 501.5, 0, 0),
(167, 97, 'Ironhew',     3, 1, 0, 60, 1537, 0, -4920.0, -958.0, 501.5, 0, 0),
(168, 97, 'Whisperleaf', 4, 11, 1, 60, 1537, 0, -4920.0, -958.0, 501.5, 0, 0),
(169, 97, 'Tizzle',      7, 8, 0, 60, 1537, 0, -4920.0, -958.0, 501.5, 0, 0);

-- Register new crowd bots in playerbot table
INSERT IGNORE INTO `playerbot` (`char_guid`, `chance`, `ai`)
VALUES
(145, 100, 'CrowdBotAI'),
(146, 100, 'CrowdBotAI'),
(147, 100, 'CrowdBotAI'),
(148, 100, 'CrowdBotAI'),
(149, 100, 'CrowdBotAI'),
(150, 100, 'CrowdBotAI'),
(151, 100, 'CrowdBotAI'),
(152, 100, 'CrowdBotAI'),
(153, 100, 'CrowdBotAI'),
(154, 100, 'CrowdBotAI'),
(155, 100, 'CrowdBotAI'),
(156, 100, 'CrowdBotAI'),
(157, 100, 'CrowdBotAI'),
(158, 100, 'CrowdBotAI'),
(159, 100, 'CrowdBotAI'),
(160, 100, 'CrowdBotAI'),
(161, 100, 'CrowdBotAI'),
(162, 100, 'CrowdBotAI'),
(163, 100, 'CrowdBotAI'),
(164, 100, 'CrowdBotAI'),
(165, 100, 'CrowdBotAI'),
(166, 100, 'CrowdBotAI'),
(167, 100, 'CrowdBotAI'),
(168, 100, 'CrowdBotAI'),
(169, 100, 'CrowdBotAI');

-- End of migration.
END IF;
END??
DELIMITER ;
CALL add_migration();
DROP PROCEDURE IF EXISTS add_migration;
