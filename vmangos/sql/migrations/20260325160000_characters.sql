DROP PROCEDURE IF EXISTS add_migration;
DELIMITER ??
CREATE PROCEDURE `add_migration`()
BEGIN
DECLARE v INT DEFAULT 1;
SET v = (SELECT COUNT(*) FROM `migrations` WHERE `id`='20260325160000');
IF v = 0 THEN
INSERT INTO `migrations` VALUES ('20260325160000');
-- Add your query below.

-- Remove pre-seeded guild data. Guild creation is now player-driven.
-- GuildBotMgr will auto-add bots to the player's guild dynamically.
DELETE FROM `guild_member` WHERE `guild_id` = 1 AND `guid` BETWEEN 200 AND 219;
DELETE FROM `guild_rank` WHERE `guild_id` = 1;
DELETE FROM `guild` WHERE `guild_id` = 1;

-- End of migration.
END IF;
END??
DELIMITER ;
CALL add_migration();
DROP PROCEDURE IF EXISTS add_migration;
