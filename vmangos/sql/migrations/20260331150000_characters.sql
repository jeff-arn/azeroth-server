DROP PROCEDURE IF EXISTS add_migration;
DELIMITER ??
CREATE PROCEDURE `add_migration`()
BEGIN
DECLARE v INT DEFAULT 1;
SET v = (SELECT COUNT(*) FROM `migrations` WHERE `id`='20260331150000');
IF v = 0 THEN
INSERT INTO `migrations` VALUES ('20260331150000');
-- Add your query below.

-- 15 crowd bots for IF AH area (GUIDs 175-189, account 97)
INSERT IGNORE INTO `characters` (`guid`, `account`, `name`, `race`, `class`, `gender`, `level`, `zone`, `map`, `position_x`, `position_y`, `position_z`, `orientation`, `money`)
VALUES
(175, 97, 'Bidsworth',    1, 1, 0, 60, 1537, 0, -4960.0, -906.0, 503.8, 0, 0),
(176, 97, 'Auctrina',     1, 8, 1, 60, 1537, 0, -4960.0, -906.0, 503.8, 0, 0),
(177, 97, 'Hammerbid',    3, 3, 0, 60, 1537, 0, -4960.0, -906.0, 503.8, 0, 0),
(178, 97, 'Lotspring',    4, 11, 1, 60, 1537, 0, -4960.0, -906.0, 503.8, 0, 0),
(179, 97, 'Nimbletrade',  7, 4, 0, 60, 1537, 0, -4960.0, -906.0, 503.8, 0, 0),
(180, 97, 'Goldwynd',     1, 5, 1, 60, 1537, 0, -4960.0, -906.0, 503.8, 0, 0),
(181, 97, 'Bronzebeard',  3, 2, 0, 60, 1537, 0, -4960.0, -906.0, 503.8, 0, 0),
(182, 97, 'Vendara',      4, 3, 1, 60, 1537, 0, -4960.0, -906.0, 503.8, 0, 0),
(183, 97, 'Coppersnap',   7, 9, 0, 60, 1537, 0, -4960.0, -906.0, 503.8, 0, 0),
(184, 97, 'Marthyn',      1, 4, 1, 60, 1537, 0, -4960.0, -906.0, 503.8, 0, 0),
(185, 97, 'Forgeprice',   3, 1, 0, 60, 1537, 0, -4960.0, -906.0, 503.8, 0, 0),
(186, 97, 'Silverlot',    4, 5, 1, 60, 1537, 0, -4960.0, -906.0, 503.8, 0, 0),
(187, 97, 'Gearworth',    7, 8, 0, 60, 1537, 0, -4960.0, -906.0, 503.8, 0, 0),
(188, 97, 'Hagglemore',   1, 9, 0, 60, 1537, 0, -4960.0, -906.0, 503.8, 0, 0),
(189, 97, 'Tinkerbell',   7, 8, 1, 60, 1537, 0, -4960.0, -906.0, 503.8, 0, 0);

INSERT IGNORE INTO `playerbot` (`char_guid`, `chance`, `ai`)
VALUES
(175, 100, 'CrowdBotAI'),
(176, 100, 'CrowdBotAI'),
(177, 100, 'CrowdBotAI'),
(178, 100, 'CrowdBotAI'),
(179, 100, 'CrowdBotAI'),
(180, 100, 'CrowdBotAI'),
(181, 100, 'CrowdBotAI'),
(182, 100, 'CrowdBotAI'),
(183, 100, 'CrowdBotAI'),
(184, 100, 'CrowdBotAI'),
(185, 100, 'CrowdBotAI'),
(186, 100, 'CrowdBotAI'),
(187, 100, 'CrowdBotAI'),
(188, 100, 'CrowdBotAI'),
(189, 100, 'CrowdBotAI');

-- End of migration.
END IF;
END??
DELIMITER ;
CALL add_migration();
DROP PROCEDURE IF EXISTS add_migration;
