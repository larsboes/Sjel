import registry from './registry.json';

export type ObjectType = typeof registry.objects[number];
export type Row = Record<string, unknown>;
export type Link = ObjectType['links'][number];
export type Binding = { node: string; object: string; read: string; parameters: Record<string, string> };

/** Resolve only declared reads. A binding never supplies a URL or method. */
export function resolveBinding(binding: Binding) {
  if (!binding || typeof binding !== 'object' || typeof binding.node !== 'string') throw new Error('binding: missing node');
  if (Object.keys(binding).some(key => !['node', 'object', 'read', 'parameters'].includes(key)) || !binding.parameters || typeof binding.parameters !== 'object' || Array.isArray(binding.parameters)) throw new Error(`${binding.node}: invalid binding shape`);
  const fail = (reason: string): never => { throw new Error(`${binding.node}: ${reason}`); };
  const object = registry.objects.find(o => o.id === binding.object) ?? fail('unknown object');
  const read = object.reads.find(r => r.id === binding.read) ?? fail('unknown read or write binding');
  if (read.method !== 'GET' || !read.path.startsWith('/api/')) fail('unapproved read');
  const keys = Object.keys(binding.parameters);
  if (keys.length !== read.parameters.length || keys.some(key => !read.parameters.some(parameter => parameter === key)) || Object.values(binding.parameters).some(v => typeof v !== 'string')) fail('invalid parameters');
  const query = new URLSearchParams();
  for (const [key, value] of Object.entries(read.fixed_parameters)) {
    if (typeof value !== 'string') fail('invalid fixed parameter');
    else query.set(key, value);
  }
  for (const [key, value] of Object.entries(binding.parameters)) query.set(key, value);
  return { object, read, url: `/${registry.capability}${read.path}?${query}` };
}

/** Keep classification attached to every row; malformed data fails closed. */
export function rowsFrom(binding: Binding, body: unknown): Row[] {
  const { object, read } = resolveBinding(binding);
  const member = body && typeof body === 'object' ? (body as Row)[read.member] : undefined;
  const rows = read.many ? member : [member];
  if (!Array.isArray(rows)) throw new Error(`${binding.node}: invalid response member`);
  for (const row of rows) {
    if (!row || typeof row !== 'object' || Array.isArray(row) || !['c0', 'c1', 'c2', 'c3'].includes(row[object.classification.field]) || object.identity.some(p => !(p in row))) throw new Error(`${binding.node}: invalid classified object`);
  }
  const schema = object.schema as WireSchema;
  for (const row of rows) {
    if (schema.required?.some(property => !(property in row)) || Object.entries(row).some(([key, value]) => !schema.properties?.[key] || !matchesWireValue(value, schema.properties[key], schema.$defs ?? {}))) throw new Error(`${binding.node}: invalid object properties`);
  }
  return rows;
}

type WireSchema = { minimum?: number; maximum?: number; type?: string | string[]; enum?: unknown[]; $ref?: string; $defs?: Record<string, WireSchema | undefined>; required?: string[]; properties?: Record<string, WireSchema | undefined> };
function matchesWireValue(value: unknown, schema: WireSchema, definitions: Record<string, WireSchema | undefined>): boolean {
  if (schema.$ref) {
    const target = definitions[schema.$ref.replace('#/$defs/', '')];
    return !!target && matchesWireValue(value, target, definitions);
  }
  if (schema.enum && !schema.enum.includes(value)) return false;
  if (typeof value === 'number' && ((schema.minimum !== undefined && value < schema.minimum) || (schema.maximum !== undefined && value > schema.maximum))) return false;
  const types = Array.isArray(schema.type) ? schema.type : [schema.type];
  return types.some(type => type === 'null' ? value === null : type === 'integer' ? typeof value === 'number' && Number.isSafeInteger(value) : typeof value === type);
}

/** Follow registry property bindings, including identity filters for aggregate reads. */
export function linkedBinding(object: ObjectType, link: Link, row: Row): Binding {
  if (!object.links.some(candidate => candidate === link)) throw new Error(`${object.id}: undeclared link`);
  return { node: `${object.id}/${link.label}`, object: link.target, read: link.read, parameters: Object.fromEntries(Object.entries(link.parameters).map(([key, property]) => [key, String(row[property])])) };
}

/** Apply the relation's declared equality constraints without interpreting domain fields. */
export function linkedRows(link: Link, parent: Row, rows: Row[]): Row[] {
  return rows.filter(row => Object.entries(link.equals).every(([key, source]) => row[key] === parent[source]));
}

export async function readRows(binding: Binding, signal?: AbortSignal): Promise<Row[]> {
  const { url } = resolveBinding(binding);
  const response = await fetch(url, { method: 'GET', credentials: 'same-origin', redirect: 'error', signal, cache: 'no-store' });
  if (!response.ok) throw new Error(`${binding.node}: read failed (${response.status})`);
  return rowsFrom(binding, await response.json());
}

export { registry };
