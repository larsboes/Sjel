PRAGMA foreign_keys=ON;
BEGIN IMMEDIATE;
INSERT INTO inventory_item(id, kind, label, created_at, updated_at)
VALUES ('sjel-turso-rehearsal-item', 'gear', 'Rehearsal fixture', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
INSERT INTO inventory_item_state(item_id, state, since)
VALUES ('sjel-turso-rehearsal-item', 'wanted', '2026-01-01T00:00:00Z');
INSERT INTO inventory_wish_group(id, title, created_at, updated_at)
VALUES ('sjel-turso-rehearsal-group', 'Rehearsal fixture', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
INSERT INTO inventory_wish_group_item(group_id, item_id)
VALUES ('sjel-turso-rehearsal-group', 'sjel-turso-rehearsal-item');
INSERT INTO inventory_wish_spec(item_id, key, value, unit)
VALUES ('sjel-turso-rehearsal-item', 'weight', 840.5, 'g');
INSERT INTO inventory_wish_group_need(group_id, key, value, unit)
VALUES ('sjel-turso-rehearsal-group', 'weight', 1000, 'g');
INSERT INTO inventory_price_observation(item_id, source, price_cents, observed_at, observed_on)
VALUES ('sjel-turso-rehearsal-item', 'rehearsal', 12999, '2026-01-01T00:00:00Z', '2026-01-01');
UPDATE inventory_item SET label = 'Revised fixture', revision = revision + 1
WHERE id = 'sjel-turso-rehearsal-item' AND revision = 1 RETURNING revision;
UPDATE inventory_item SET label = 'Stale write', revision = revision + 1
WHERE id = 'sjel-turso-rehearsal-item' AND revision = 1 RETURNING revision;
COMMIT;

-- Rehearse a proposed atomic application and retry. The current HTTP handler uses separate writes.
BEGIN IMMEDIATE;
UPDATE inventory_item SET label = 'Acknowledged fixture', revision = revision + 1
WHERE id = 'sjel-turso-rehearsal-item' AND revision = 2
AND NOT EXISTS (SELECT 1 FROM inventory_sync_operation WHERE operation_id = 'sjel-turso-rehearsal-op')
RETURNING revision;
INSERT INTO inventory_sync_operation(operation_id, revision, processed_at)
VALUES ('sjel-turso-rehearsal-op', 3, '2026-01-02T00:00:00Z');
INSERT INTO inventory_item_state(item_id, state, since)
VALUES ('sjel-turso-rehearsal-item', 'owned', '2026-01-02T00:00:00Z');
COMMIT;
BEGIN IMMEDIATE;
UPDATE inventory_item SET label = 'Duplicate replay', revision = revision + 1
WHERE id = 'sjel-turso-rehearsal-item' AND revision = 3
AND NOT EXISTS (SELECT 1 FROM inventory_sync_operation WHERE operation_id = 'sjel-turso-rehearsal-op')
RETURNING revision;
COMMIT;

SELECT i.revision, i.label,
    (SELECT state FROM inventory_item_state WHERE item_id=i.id ORDER BY since DESC, id DESC LIMIT 1),
    (SELECT count(*) FROM inventory_item_state WHERE item_id=i.id),
    (SELECT revision FROM inventory_sync_operation WHERE operation_id='sjel-turso-rehearsal-op'),
    (SELECT group_id FROM inventory_wish_group_item WHERE item_id=i.id),
    (SELECT price_cents FROM inventory_price_observation WHERE item_id=i.id)
FROM inventory_item i WHERE i.id='sjel-turso-rehearsal-item';
