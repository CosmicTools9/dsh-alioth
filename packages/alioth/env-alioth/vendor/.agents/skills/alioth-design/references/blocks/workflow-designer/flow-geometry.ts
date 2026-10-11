/**
 * workflow-designer — 画布几何计算
 * calcEP / elbowPath 与生产 FlowDesigner.tsx 同名函数对齐
 */
import type { FlowNode, FlowEdge } from './flow-types';

export function getSize(t: string) {
  const sizes: Record<string, { w: number; h: number }> = {
    start: { w: 90, h: 44 }, end: { w: 90, h: 44 },
    condition: { w: 70, h: 70 }, parallel: { w: 70, h: 70 }, branch: { w: 70, h: 70 },
    approval: { w: 180, h: 90 }, cc: { w: 160, h: 70 }, subflow: { w: 160, h: 70 },
  };
  return sizes[t] ?? { w: 140, h: 80 };
}

/** Port endpoint calc (aligns production calcEndpoints) */
export function calcEP(fx: number, fy: number, fw: number, fh: number, tx: number, ty: number, tw: number, th: number) {
  const fcx = fx + fw / 2, fcy = fy + fh / 2, tcx = tx + tw / 2, tcy = ty + th / 2;
  const dx = tcx - fcx, dy = tcy - fcy;
  let fs: string, ts: string;
  if (Math.abs(dx) >= Math.abs(dy)) { fs = dx >= 0 ? 'right' : 'left'; ts = dx >= 0 ? 'left' : 'right'; }
  else { fs = dy >= 0 ? 'bottom' : 'top'; ts = dy >= 0 ? 'top' : 'bottom'; }
  const port = (s: string, px: number, py: number, pw: number, ph: number): [number, number] => {
    if (s === 'top') return [px + pw / 2, py];
    if (s === 'right') return [px + pw, py + ph / 2];
    if (s === 'bottom') return [px + pw / 2, py + ph];
    return [px, py + ph / 2];
  };
  const [sx, sy] = port(fs, fx, fy, fw, fh);
  const [ex, ey] = port(ts, tx, ty, tw, th);
  return { sx, sy, ex, ey, fs, ts };
}

/** Elbow path (aligns production elbowPath) */
export function elbowPath(sx: number, sy: number, ex: number, ey: number, fs?: string, ts?: string): string {
  const O = 28;
  const dx = fs === 'right' ? sx + O : fs === 'left' ? sx - O : sx;
  const dy = fs === 'bottom' ? sy + O : fs === 'top' ? sy - O : sy;
  const ax = ts === 'left' ? ex - O : ts === 'right' ? ex + O : ex;
  const ay = ts === 'top' ? ey - O : ts === 'bottom' ? ey + O : ey;
  return 'M ' + sx + ' ' + sy + ' L ' + dx + ' ' + dy + ' L ' + ax + ' ' + dy + ' L ' + ax + ' ' + ay + ' L ' + ex + ' ' + ey;
}

/** 计算 elbow 路径的中点坐标（用于标签定位） */
export function elbowMidpoint(sx: number, sy: number, ex: number, ey: number, fs?: string, ts?: string): { x: number; y: number } {
  const O = 28;
  const dx = fs === 'right' ? sx + O : fs === 'left' ? sx - O : sx;
  const dy = fs === 'bottom' ? sy + O : fs === 'top' ? sy - O : sy;
  const ax = ts === 'left' ? ex - O : ts === 'right' ? ex + O : ex;
  const ay = ts === 'top' ? ey - O : ts === 'bottom' ? ey + O : ey;
  const segs: Array<{ x0: number; y0: number; x1: number; y1: number }> = [
    { x0: sx, y0: sy, x1: dx, y1: dy },
    { x0: dx, y0: dy, x1: ax, y1: dy },
    { x0: ax, y0: dy, x1: ax, y1: ay },
    { x0: ax, y0: ay, x1: ex, y1: ey },
  ];
  const totalLen = segs.reduce((s, seg) => s + Math.abs(seg.x1 - seg.x0) + Math.abs(seg.y1 - seg.y0), 0);
  if (totalLen <= 0) return { x: (sx + ex) / 2, y: (sy + ey) / 2 };
  const half = totalLen / 2;
  let acc = 0;
  for (const seg of segs) {
    const segLen = Math.abs(seg.x1 - seg.x0) + Math.abs(seg.y1 - seg.y0);
    if (acc + segLen >= half || segLen === 0) {
      const t = segLen > 0 ? (half - acc) / segLen : 0;
      return { x: seg.x0 + (seg.x1 - seg.x0) * t, y: seg.y0 + (seg.y1 - seg.y0) * t };
    }
    acc += segLen;
  }
  return { x: (sx + ex) / 2, y: (sy + ey) / 2 };
}

export interface Connection {
  fromIdx: number;
  toIdx: number;
  sx: number; sy: number; ex: number; ey: number;
  fs: string; ts: string;
  /** 连线标签 */
  label?: string;
  /** 标签渲染位置（路径中点） */
  labelX: number;
  labelY: number;
}
/** 由 nodes + edges 计算全部连线的几何端点和标签位置 */
export function buildConnections(nodes: FlowNode[], edges: FlowEdge[]): Connection[] {
  const idxById = new Map(nodes.map((n, i) => [n.id, i]));
  const list: Connection[] = [];
  for (const e of edges) {
    const fi = idxById.get(e.from);
    const ti = idxById.get(e.to);
    if (fi === undefined || ti === undefined) continue;
    const f = nodes[fi], t = nodes[ti];
    const fsz = getSize(f.type), tsz = getSize(t.type);
    const ep = calcEP(f.x, f.y, fsz.w, fsz.h, t.x, t.y, tsz.w, tsz.h);
    const mp = elbowMidpoint(ep.sx, ep.sy, ep.ex, ep.ey, ep.fs, ep.ts);
    list.push({
      fromIdx: fi, toIdx: ti,
      sx: ep.sx, sy: ep.sy, ex: ep.ex, ey: ep.ey,
      fs: ep.fs, ts: ep.ts,
      label: e.label,
      labelX: mp.x,
      labelY: mp.y,
    });
  }
  return list;
}

/** 顺序连接的 fallback：无显式 edges 时按数组顺序 i → i+1 */
export function sequentialEdges(nodes: FlowNode[]): FlowEdge[] {
  const edges: FlowEdge[] = [];
  for (let i = 0; i < nodes.length - 1; i++) {
    edges.push({ from: nodes[i].id, to: nodes[i + 1].id });
  }
  return edges;
}
