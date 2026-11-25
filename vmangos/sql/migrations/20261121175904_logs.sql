DROP PROCEDURE IF EXISTS add_migration;
DELIMITER ??
CREATE PROCEDURE `add_migration`()
BEGIN
DECLARE v INT DEFAULT 1;
SET v = (SELECT COUNT(*) FROM `migrations` WHERE `id`='20261121175904');
IF v = 0 THEN
INSERT INTO `migrations` VALUES ('20261121175904');
-- Add your query below.

CREATE TABLE IF NOT EXISTS `client_report_pair_codes` (
  `code_hash` char(64) NOT NULL,
  `account_id` int(10) unsigned NOT NULL,
  `account_name` varchar(32) NOT NULL,
  `expires_at` datetime NOT NULL,
  `used_at` datetime NULL DEFAULT NULL,
  `created_at` datetime NOT NULL,
  PRIMARY KEY (`code_hash`),
  KEY `idx_client_report_pair_account` (`account_id`),
  KEY `idx_client_report_pair_expires` (`expires_at`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='One-time codes pairing a client reporter install to an account';

CREATE TABLE IF NOT EXISTS `client_report_installs` (
  `id` char(36) NOT NULL,
  `account_id` int(10) unsigned NOT NULL,
  `account_name` varchar(32) NOT NULL,
  `token_hash` char(64) NOT NULL,
  `label` varchar(120) NULL DEFAULT NULL,
  `created_at` datetime NOT NULL,
  `last_seen_at` datetime NULL DEFAULT NULL,
  `revoked_at` datetime NULL DEFAULT NULL,
  PRIMARY KEY (`id`),
  UNIQUE KEY `uq_client_report_install_token` (`token_hash`),
  KEY `idx_client_report_install_account` (`account_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='Paired client reporter installations';

CREATE TABLE IF NOT EXISTS `client_reports` (
  `id` char(36) NOT NULL,
  `install_id` char(36) NOT NULL,
  `account_id` int(10) unsigned NOT NULL,
  `account_name` varchar(32) NOT NULL,
  `client_report_id` varchar(128) NOT NULL,
  `kind` varchar(16) NOT NULL,
  `title` varchar(200) NOT NULL,
  `body` text NULL,
  `severity` varchar(16) NOT NULL DEFAULT 'normal',
  `status` varchar(16) NOT NULL DEFAULT 'new',
  `occurred_at` datetime NOT NULL,
  `received_at` datetime NOT NULL,
  `character_name` varchar(64) NULL DEFAULT NULL,
  `realm_name` varchar(64) NULL DEFAULT NULL,
  `pack_name` varchar(64) NULL DEFAULT NULL,
  `pack_version` varchar(32) NULL DEFAULT NULL,
  `client_build` int(11) NULL DEFAULT NULL,
  `client_sha256` varchar(64) NULL DEFAULT NULL,
  `context_json` longtext NOT NULL,
  PRIMARY KEY (`id`),
  UNIQUE KEY `uq_client_report_install_client_id` (`install_id`, `client_report_id`),
  KEY `idx_client_report_status_received` (`status`, `received_at`),
  KEY `idx_client_report_account_received` (`account_id`, `received_at`),
  CONSTRAINT `fk_client_reports_install`
    FOREIGN KEY (`install_id`) REFERENCES `client_report_installs` (`id`) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='Client bug, Lua error, and crash reports';

CREATE TABLE IF NOT EXISTS `client_report_files` (
  `id` char(36) NOT NULL,
  `report_id` char(36) NOT NULL,
  `filename` varchar(255) NOT NULL,
  `content_type` varchar(120) NOT NULL DEFAULT 'text/plain',
  `size_bytes` int(10) unsigned NOT NULL DEFAULT 0,
  `sha256` char(64) NOT NULL,
  `storage_path` varchar(512) NOT NULL,
  PRIMARY KEY (`id`),
  KEY `idx_client_report_files_report` (`report_id`),
  CONSTRAINT `fk_client_report_files_report`
    FOREIGN KEY (`report_id`) REFERENCES `client_reports` (`id`) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='Redacted text attachments for client reports';

-- End of migration.
END IF;
END??
DELIMITER ;
CALL add_migration();
DROP PROCEDURE IF EXISTS add_migration;
