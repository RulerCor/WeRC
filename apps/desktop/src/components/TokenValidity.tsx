import { useState } from 'react';

const presets = [[60, '1 分钟'], [300, '5 分钟'], [3600, '1 小时'], [86400, '1 天'], [604800, '7 天'], [2592000, '30 天'], [7776000, '90 天']] as const;
const units = [[1, '秒'], [60, '分钟'], [3600, '小时'], [86400, '天']] as const;
export const permanentTokenExpiry = '9999-12-31T23:59:59+00:00';

type Validity = { kind: 'permanent' } | { kind: 'timed'; seconds: number };

export function useTokenValidity(defaultDays: 7 | 90, disabled: boolean, fieldClass?: string) {
  const [choice, setChoice] = useState<string>();
  const [amount, setAmount] = useState('');
  const [unit, setUnit] = useState(60);
  const selected = choice ?? String(defaultDays * 86400);
  const seconds = selected === 'custom' ? Number(amount) * unit : Number(selected);
  const validity: Validity | undefined = selected === 'permanent'
    ? { kind: 'permanent' }
    : Number.isSafeInteger(seconds) && seconds > 0 && (selected !== 'custom' || Number.isInteger(Number(amount)))
      ? { kind: 'timed', seconds }
      : undefined;
  const fields = <>
    <label className={fieldClass}>有效期<select value={selected} disabled={disabled} onChange={e => setChoice(e.target.value)}>
      {presets.map(([value, label]) => <option key={value} value={value}>{label}</option>)}
      <option value="permanent">永久</option>
      <option value="custom">自定义</option>
    </select></label>
    {selected === 'custom' && <>
      <label className={fieldClass}>时长<input type="number" min={1} step={1} required value={amount} disabled={disabled} onChange={e => setAmount(e.target.value)}/></label>
      <label className={fieldClass}>单位<select value={unit} disabled={disabled} onChange={e => setUnit(Number(e.target.value))}>
        {units.map(([value, label]) => <option key={value} value={value}>{label}</option>)}
      </select></label>
    </>}
  </>;
  return { validity, fields };
}
