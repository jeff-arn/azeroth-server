DROP PROCEDURE IF EXISTS add_migration;
DELIMITER ??
CREATE PROCEDURE `add_migration`()
BEGIN
DECLARE v INT DEFAULT 1;
SET v = (SELECT COUNT(*) FROM `migrations` WHERE `id`='20260331140000');
IF v = 0 THEN
INSERT INTO `migrations` VALUES ('20260331140000');
-- Add your query below.

-- 5 crowd bots for IF bank interior (GUIDs 170-174, account 97)
INSERT IGNORE INTO `characters` (`guid`, `account`, `name`, `race`, `class`, `gender`, `level`, `zone`, `map`, `position_x`, `position_y`, `position_z`, `orientation`, `money`)
VALUES
(170, 97, 'Vaultkeeper',  3, 1, 0, 60, 1537, 0, -4891.7, -990.8, 503.9, 0, 0),
(171, 97, 'Goleli',       1, 8, 1, 60, 1537, 0, -4891.7, -990.8, 503.9, 0, 0),
(172, 97, 'Coinsworth',   7, 4, 0, 60, 1537, 0, -4891.7, -990.8, 503.9, 0, 0),
(173, 97, 'Delvara',      4, 11, 1, 60, 1537, 0, -4891.7, -990.8, 503.9, 0, 0),
(174, 97, 'Stoutledger',  3, 2, 0, 60, 1537, 0, -4891.7, -990.8, 503.9, 0, 0);

INSERT IGNORE INTO `playerbot` (`char_guid`, `chance`, `ai`)
VALUES
(170, 100, 'CrowdBotAI'),
(171, 100, 'CrowdBotAI'),
(172, 100, 'CrowdBotAI'),
(173, 100, 'CrowdBotAI'),
(174, 100, 'CrowdBotAI');

-- End of migration.
END IF;
END??
DELIMITER ;
CALL add_migration();
DROP PROCEDURE IF EXISTS add_migration;
