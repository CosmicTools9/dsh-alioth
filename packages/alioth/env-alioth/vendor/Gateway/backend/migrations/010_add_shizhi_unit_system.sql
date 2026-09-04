--! no-transaction
-- Add 市制 (Chinese customary units) to the unit system enum.
-- ALTER TYPE ... ADD VALUE cannot run inside a transaction block.
ALTER TYPE isahl.zc_id_unit_system_enum ADD VALUE IF NOT EXISTS '市制';
