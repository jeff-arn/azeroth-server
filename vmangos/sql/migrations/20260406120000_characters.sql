DROP PROCEDURE IF EXISTS add_migration;
DELIMITER ??
CREATE PROCEDURE `add_migration`()
BEGIN
DECLARE v INT DEFAULT 1;
SET v = (SELECT COUNT(*) FROM `migrations` WHERE `id`='20260406120000');
IF v = 0 THEN
INSERT INTO `migrations` VALUES ('20260406120000');

CREATE TABLE IF NOT EXISTS `bot_learned_patterns` (
    `creature_entry` INT UNSIGNED NOT NULL,
    `trigger_spell_id` INT UNSIGNED NOT NULL,
    `response_type` TINYINT UNSIGNED NOT NULL DEFAULT 0 COMMENT '0=interrupt,1=move_away,2=spread,3=save_cd',
    `response_param` FLOAT NOT NULL DEFAULT 0 COMMENT 'e.g. avoidance range',
    `confidence` TINYINT UNSIGNED NOT NULL DEFAULT 1,
    `is_permanent` TINYINT UNSIGNED NOT NULL DEFAULT 0 COMMENT '1=hand-crafted override, cannot decay',
    `last_seen` INT UNSIGNED NOT NULL DEFAULT 0 COMMENT 'unix timestamp',
    PRIMARY KEY (`creature_entry`, `trigger_spell_id`, `response_type`),
    KEY `idx_creature` (`creature_entry`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8;

END IF;
END??
DELIMITER ;
CALL add_migration();
DROP PROCEDURE IF EXISTS add_migration;
