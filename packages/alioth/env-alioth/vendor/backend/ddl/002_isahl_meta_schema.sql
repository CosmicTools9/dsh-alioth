-- AppCreator standalone: isahl_meta schema seed
-- Auto-loaded by AppCreator at startup if isahl_meta schema does not exist.
-- Schema only — no business seed data (see separate isahl_meta-seed-data.sql if needed).
--
-- Source: aliothstudio_dev (2026-07-27), pg_dump -s
-- Only the tables/views/types that vendored AppAgent reads.

-- ── ENUM types ──────────────────────────────────────────────────────────

CREATE TYPE isahl_meta.collection_type AS ENUM (
    'table',
    'view',
    'materialized_view',
    'external'
);

CREATE TYPE isahl_meta.field_category AS ENUM (
    'scalar',
    'reference',
    'computed',
    'auto'
);

CREATE TYPE isahl_meta.field_data_type AS ENUM (
    'text',
    'integer',
    'decimal',
    'boolean',
    'time',
    'timestamptz',
    'jsonb',
    'array',
    'bigint',
    'uuid',
    'enum',
    'h1',
    'm2o',
    'o2o',
    'hm',
    'm2n',
    'combine',
    'transform',
    'm2m'
);

-- ── Tables ──────────────────────────────────────────────────────────────

CREATE TABLE isahl_meta.meta_collections (
    table_name text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    created_by_id bigint DEFAULT 1,
    updated_by_id bigint DEFAULT 1,
    name text NOT NULL,
    type isahl_meta.collection_type,
    config jsonb DEFAULT '{}'::jsonb,
    data_source text,
    schema text DEFAULT 'isahl'::text,
    biz_description text
);

ALTER TABLE isahl_meta.meta_collections ADD PRIMARY KEY (table_name);
CREATE INDEX idx_meta_collections_type ON isahl_meta.meta_collections (type);

CREATE TABLE isahl_meta.meta_fields (
    fk_collection text NOT NULL REFERENCES isahl_meta.meta_collections(table_name) ON DELETE CASCADE,
    name text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    created_by_id bigint DEFAULT 1,
    updated_by_id bigint DEFAULT 1,
    category isahl_meta.field_category,
    data_type isahl_meta.field_data_type,
    is_required boolean DEFAULT false,
    default_value text,
    config jsonb DEFAULT '{}'::jsonb,
    title text NOT NULL DEFAULT ''::text
);

ALTER TABLE isahl_meta.meta_fields ADD PRIMARY KEY (fk_collection, name);
CREATE INDEX idx_meta_field_collection ON isahl_meta.meta_fields (fk_collection);
CREATE INDEX idx_meta_field_name ON isahl_meta.meta_fields (name);

-- ── Views ───────────────────────────────────────────────────────────────

-- devv_inherits_view: lists all isahl schema tables and their parent
-- (via pg_inherits). Works against any database with isahl.* tables.
CREATE VIEW isahl_meta.devv_inherits_view AS
SELECT
    row_number() OVER (ORDER BY p.relname, c.relname) AS id,
    p.relname AS parent,
    c.relname AS sub
FROM pg_class c
JOIN pg_namespace cn ON cn.oid = c.relnamespace
LEFT JOIN pg_inherits ON pg_inherits.inhrelid = c.oid
LEFT JOIN pg_class p ON p.oid = pg_inherits.inhparent
LEFT JOIN pg_namespace pn ON pn.oid = p.relnamespace
WHERE cn.nspname = 'isahl'::name AND c.relkind = 'r'::"char"
ORDER BY p.relname, c.relname;

-- devv_inherits_union: recursive view of the full inheritance tree.
-- Depends on devv_inherits_view above.
CREATE VIEW isahl_meta.devv_inherits_union AS
WITH RECURSIVE tree(sub, parent, depth, path) AS (
    SELECT i.sub,
        '-'::name AS parent,
        0 AS depth,
        i.parent::text AS path
    FROM isahl_meta.devv_inherits_view i
    WHERE i.parent IS NULL
    UNION ALL
    SELECT i.sub,
        i.parent,
        tree.depth + 1,
        (COALESCE(tree.path, tree.sub::text) || ' → '::text) || i.sub::text AS path
    FROM tree
    JOIN isahl_meta.devv_inherits_view i ON tree.sub = i.parent
    WHERE i.parent <> 'zc_id_object'::name
)
SELECT
    row_number() OVER (ORDER BY depth, parent, sub) AS id,
    sub,
    parent,
    depth,
    path
FROM tree
ORDER BY depth, parent, sub;

-- ── Helper functions ────────────────────────────────────────────────────

CREATE OR REPLACE FUNCTION isahl_meta.gf_is_inherits_of(sub_t name, parent_t text)
RETURNS boolean LANGUAGE plpgsql AS $function$
BEGIN
    RETURN sub_t = parent_t OR EXISTS (
        WITH RECURSIVE sub (inhrelid, inhparent, child, parent) AS (
            SELECT pi.inhrelid, pi.inhparent, c.relname AS child, p.relname AS parent
            FROM pg_inherits AS pi
            JOIN pg_class AS c ON pi.inhrelid = c.oid
            JOIN pg_class AS p ON pi.inhparent = p.oid
            WHERE c.relname = sub_t
            UNION ALL
            SELECT pi.inhrelid, pi.inhparent, sub.child, c.relname AS parent
            FROM sub
            JOIN (pg_inherits AS pi JOIN pg_class AS c ON c.oid = pi.inhparent) ON pi.inhrelid = sub.inhparent
        )
        SELECT sub.parent, sub.child FROM sub WHERE sub.parent = parent_t
    );
END;
$function$;

CREATE OR REPLACE FUNCTION isahl_meta.gf_query_inherits(src name, exself boolean DEFAULT false)
RETURNS TABLE(sub name, depth integer) LANGUAGE plpgsql AS $function$
BEGIN
    RETURN QUERY
    WITH RECURSIVE tree (sub, parent, depth) AS (
        SELECT i.sub, i.parent, 0 AS depth
        FROM isahl_meta.devv_inherits_view AS i
        WHERE i.sub = src
        UNION ALL
        SELECT i.sub, i.parent, tree.depth + 1
        FROM tree
        JOIN isahl_meta.devv_inherits_view AS i ON tree.sub = i.parent
    ), dis AS(
        SELECT tree.sub, MAX(tree.depth) AS depth FROM tree GROUP BY tree.sub
    )
    SELECT dis.sub, dis.depth FROM dis WHERE (exself AND dis.sub != src) OR NOT exself ORDER BY dis.depth, dis.sub;
END;
$function$;

CREATE OR REPLACE FUNCTION isahl_meta.gf_query_inherits(src text, exself boolean DEFAULT false)
RETURNS TABLE(sub name, depth integer) LANGUAGE plpgsql STABLE AS $function$
BEGIN
    RETURN QUERY SELECT s.sub, s.depth
    FROM isahl_meta.gf_query_inherits(src::name, exself) s;
END;
$function$;

CREATE OR REPLACE FUNCTION isahl_meta.gf_query_leafs(parent_name name, OUT leafs name[])
RETURNS name[] LANGUAGE plpgsql AS $function$
BEGIN
    WITH RECURSIVE tree (sub, parent) AS (
        SELECT i.sub, i.parent FROM isahl_meta.devv_inherits_view AS i
        WHERE i.parent = parent_name
        UNION ALL
        SELECT i.sub, i.parent FROM tree
        JOIN isahl_meta.devv_inherits_view AS i ON i.parent = tree.sub
    ), leaf_nodes AS (
        SELECT DISTINCT tree.sub FROM tree
        WHERE tree.sub NOT IN (
            SELECT parent FROM isahl_meta.devv_inherits_view WHERE parent IS NOT NULL
        )
    )
    SELECT array_agg(sub ORDER BY sub) FROM leaf_nodes INTO leafs;
END;
$function$;

CREATE OR REPLACE FUNCTION isahl_meta.gf_query_parent_of(src name, OUT parents name[])
RETURNS name[] LANGUAGE plpgsql AS $function$
BEGIN
    SELECT COALESCE(array_agg(parent),'{}')
    FROM isahl_meta.devv_inherits_view
    WHERE sub = src AND parent IS NOT NULL
    INTO parents;
END;
$function$;

CREATE OR REPLACE FUNCTION isahl_meta.gf_query_parents_of(src name, OUT parents name[])
RETURNS name[] LANGUAGE plpgsql AS $function$
BEGIN
    WITH RECURSIVE tree (sub, parent) AS (
        SELECT i.sub, i.parent FROM isahl_meta.devv_inherits_view AS i
        WHERE i.sub = src
        UNION ALL
        SELECT i.sub, i.parent FROM tree
        JOIN isahl_meta.devv_inherits_view AS i ON i.sub = tree.parent
    ), dict AS (
        SELECT DISTINCT tree.parent FROM tree
    )
    SELECT array_agg(parent) FROM dict WHERE parent IS NOT NULL INTO parents;
END;
$function$;

CREATE OR REPLACE FUNCTION isahl_meta.gf_query_subs_of(src name, only_leaf boolean DEFAULT false, OUT subs name[])
RETURNS name[] LANGUAGE plpgsql AS $function$
BEGIN
    WITH RECURSIVE tree (sub, parent) AS (
        SELECT i.sub, i.parent FROM isahl_meta.devv_inherits_view AS i
        WHERE i.parent = src
        UNION ALL
        SELECT i.sub, i.parent FROM tree
        JOIN isahl_meta.devv_inherits_view AS i ON i.parent = tree.sub
    ), dis AS(
        SELECT DISTINCT tree.sub FROM tree
        WHERE (NOT only_leaf OR tree.sub NOT IN (SELECT parent FROM isahl_meta.devv_inherits_view WHERE parent IS NOT NULL))
    )
    SELECT array_agg(sub) FROM dis INTO subs;
END;
$function$;

-- ── Grant: give the connecting user read access ─────────────────────────
GRANT USAGE ON SCHEMA isahl_meta TO PUBLIC;
GRANT SELECT ON ALL TABLES IN SCHEMA isahl_meta TO PUBLIC;
GRANT SELECT ON ALL SEQUENCES IN SCHEMA isahl_meta TO PUBLIC;
