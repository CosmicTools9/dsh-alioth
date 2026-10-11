import { useState } from 'react';
import { Page, PageHeader, MetricCard, Badge, ProgressBar, DataTable } from '../_kit/ui';
import { createPrototypeLifecycle } from '../_kit/lifecycle';
import { t } from '../_kit/i18n';

interface WF { name: string; instances: number; avgHours: number; maxHours: number; passRate: number; trend: string; }
const MOCK: WF[] = [
  { name: '标准三级审批', instances: 128, avgHours: 18, maxHours: 72, passRate: 92, trend: 'up' },
  { name: '条件分支审批', instances: 64, avgHours: 24, maxHours: 96, passRate: 85, trend: 'stable' },
  { name: '会签审批', instances: 32, avgHours: 48, maxHours: 168, passRate: 78, trend: 'down' },
  { name: '快速审批', instances: 256, avgHours: 2, maxHours: 8, passRate: 98, trend: 'up' },
  { name: '变更审批', instances: 48, avgHours: 36, maxHours: 120, passRate: 88, trend: 'stable' },
];

export default function Block() {
  const [items] = useState(MOCK);
  const total = items.reduce((s, w) => s + w.instances, 0);
  const headers = ['流程名称', '实例数', '平均时长', '最长时长', '通过率', '趋势'];
  const rows = items.map((w) => [
    w.name, String(w.instances), w.avgHours + 'h', String(w.maxHours) + 'h',
    <ProgressBar value={w.passRate} />,
    <Badge text={w.trend === 'up' ? '上升' : w.trend === 'down' ? '下降' : '持平'} variant={w.trend === 'up' ? 'success' : w.trend === 'down' ? 'danger' : 'default'} />,
  ]);
  return (
    <Page>
      <PageHeader title={t('avic.systemApprove.block.workflowAnalytics')} desc="审批流程效率与执行分析" />
      <div className="flex gap-3 mb-4">
        <MetricCard label="总审批数" value={String(total)} />
        <MetricCard label="平均时长" value={(items.reduce((s, w) => s + w.avgHours, 0) / items.length).toFixed(0) + 'h'} />
        <MetricCard label="通过率" value={(items.reduce((s, w) => s + w.passRate, 0) / items.length).toFixed(0) + '%'} />
        <MetricCard label="活跃流程" value={String(items.length)} />
      </div>
      <DataTable headers={headers} rows={rows} />
    </Page>
  );
}
Block.displayName = 'workflow-analytics';
export const { bootstrap, mount, unmount } = createPrototypeLifecycle({ name: 'workflow-analytics', App: Block });
