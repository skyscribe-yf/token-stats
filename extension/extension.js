// quota-widget@token-stats — 面板订阅配额小组件
// 数据来自 token-stats 后端：/api/quota + /api/xunfei + /api/ainaiba-credit
// 特性：Tab 切换订阅卡、隐藏订阅（GSettings 持久化）、定时/打开/手动刷新

import GObject from 'gi://GObject';
import GLib from 'gi://GLib';
import Gio from 'gi://Gio';
import Soup from 'gi://Soup?version=3';
import St from 'gi://St';
import Clutter from 'gi://Clutter';
import Cairo from 'gi://Cairo';

import {Extension} from 'resource:///org/gnome/shell/extensions/extensionUtils.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';

// ─── 卡片定义 ────────────────────────────────────────────────────────────────
// type 用于共享渲染器 / 摘要百分比函数；id 用于隐藏偏好持久化（勿改名）

const CARD_DEFS = [
    {id: 'kimi',           type: 'kimi',        name: 'Kimi'},
    {id: 'kimi-ex',        type: 'kimi',        name: 'Kimi EX'},
    {id: 'opencode-go',    type: 'opencode',    name: 'OpenCode Go'},
    {id: 'opencode-go-ex', type: 'opencode',    name: 'OpenCode EX'},
    {id: 'xiaomi-mimo',    type: 'xiaomi',      name: 'MiMo'},
    {id: 'commandcode',    type: 'commandcode', name: 'Command Code'},
    {id: 'commandcode-ex', type: 'commandcode', name: 'Cmd Code EX'},
    {id: 'ollama',         type: 'ollama',      name: 'Ollama'},
    {id: 'meituan',        type: 'meituan',     name: 'LongCat'},
    {id: 'fenno',          type: 'fenno',       name: 'Fenno'},
    {id: 'fenno-ex',       type: 'fenno',       name: 'Fenno EX'},
    {id: 'grok',           type: 'grok',        name: 'Grok'},
    {id: 'dimagent',       type: 'dimagent',    name: 'DimAgent'},
    {id: 'xunfei',         type: 'xunfei',      name: '讯飞'},
    {id: 'ainaiba',        type: 'ainaiba',     name: 'Ainaba'},
];

// ─── 格式化助手 ──────────────────────────────────────────────────────────────

const fmtC = n => { // 大数字紧凑显示（中文单位）
    if (n == null || !isFinite(n)) return '—';
    const abs = Math.abs(n);
    if (abs >= 1e8) return (n / 1e8).toFixed(2) + '亿';
    if (abs >= 1e4) return (n / 1e4).toFixed(1) + '万';
    return Math.round(n).toLocaleString('en-US');
};

const fmtF = n => (n == null || !isFinite(n)) ? '—'
    : Number(n).toLocaleString('en-US', {maximumFractionDigits: 1});

const fmtMoney = (n, cur = '¥') => (n == null || !isFinite(n)) ? '—'
    : `${cur}${Number(n).toFixed(2)}`;

function _parseIso(iso) {
    const dt = GLib.DateTime.new_from_iso8601(iso, GLib.TimeZone.new_utc());
    return dt ? dt.to_local() : null;
}

const fmtDate = iso => {
    if (!iso) return '—';
    try {
        const dt = _parseIso(iso);
        return dt ? dt.format('%m-%d %H:%M') : iso;
    } catch {
        return iso;
    }
};

// 相对时间："3 天 2 小时" / 已重置
const fmtRel = iso => {
    if (!iso) return null;
    try {
        const dt = _parseIso(iso);
        if (!dt) return null;
        const diffUs = dt.diff(GLib.DateTime.new_now_local());
        if (diffUs <= 0) return '已重置';
        const min = Math.floor(diffUs / GLib.TIME_SPAN_MINUTE);
        if (min < 1) return '即将重置';
        if (min < 60) return `${min} 分钟`;
        const h = Math.floor(min / 60);
        if (h < 48) return `${h} 小时 ${min % 60} 分`;
        return `${Math.floor(h / 24)} 天 ${h % 24} 小时`;
    } catch {
        return null;
    }
};

const fmtReset = iso => {
    const rel = fmtRel(iso);
    if (!rel) return null;
    return rel === '已重置' ? '已重置' : `${rel}后重置`;
};

const pctLevel = p => p == null ? 'none' : (p > 90 ? 'low' : p >= 75 ? 'mid' : 'ok');

const limPct = (used, limit) => (limit && limit > 0) ? used / limit * 100 : null;

const maxOf = arr => {
    const nums = arr.filter(v => v != null && isFinite(v));
    return nums.length ? Math.max(...nums) : null;
};

// ─── 进度条（Cairo 自绘，St 无 ProgressBar）──────────────────────────────────

const LEVEL_COLORS = {
    ok: [0.063, 0.725, 0.506],   // #10b981
    mid: [0.961, 0.620, 0.043],  // #f59e0b
    low: [0.957, 0.247, 0.369],  // #f43f5e
    none: [0.42, 0.45, 0.50],
};

function _roundRect(cr, x, y, w, h, r) {
    r = Math.min(r, w / 2, h / 2);
    cr.newSubPath();
    cr.arc(x + w - r, y + r, r, -Math.PI / 2, 0);
    cr.arc(x + w - r, y + h - r, r, 0, Math.PI / 2);
    cr.arc(x + r, y + h - r, r, Math.PI / 2, Math.PI);
    cr.arc(x + r, y + r, r, Math.PI, 3 * Math.PI / 2);
    cr.closePath();
}

const QProgressBar = GObject.registerClass(
class QProgressBar extends St.DrawingArea {
    _init({pct = 0, level = 'ok'} = {}) {
        super._init({x_expand: true, style: 'height: 8px;'});
        this._pct = Math.max(0, Math.min(100, pct || 0));
        this._level = level;
    }

    setValues(pct, level) {
        this._pct = Math.max(0, Math.min(100, pct || 0));
        this._level = level;
        this.queue_repaint();
    }

    vfunc_repaint() {
        const cr = this.get_context();
        const [w, h] = this.get_surface_size();
        if (w < 2 || h < 2) {
            cr.$dispose();
            return;
        }
        const r = h / 2;
        cr.setSourceRgba(1, 1, 1, 0.14);
        _roundRect(cr, 0, 0, w, h, r);
        cr.fill();
        if (this._pct > 0) {
            const c = LEVEL_COLORS[this._level] || LEVEL_COLORS.ok;
            cr.setSourceRgb(c[0], c[1], c[2]);
            _roundRect(cr, 0, 0, Math.max(h, w * this._pct / 100), h, r);
            cr.fill();
        }
        cr.$dispose();
    }
});

// ─── 面板指示器 ──────────────────────────────────────────────────────────────

const QuotaIndicator = GObject.registerClass(
class QuotaIndicator extends PanelMenu.Button {
    _init() {
        super._init(0.0, '订阅配额', false);
        const box = new St.BoxLayout({vertical: false, style_class: 'qw-indicator-box'});
        this._icon = new St.Icon({
            icon_name: 'battery-good-symbolic',
            style_class: 'system-status-icon qw-indicator-icon',
        });
        this._label = new St.Label({
            text: '…',
            y_align: Clutter.ActorAlign.CENTER,
            style_class: 'qw-indicator-label',
        });
        box.add_child(this._icon);
        box.add_child(this._label);
        this.add_child(box);
    }

    // pct: 所有可见订阅中最高的用量百分比；null 表示无数据
    setSummary(pct) {
        if (pct == null || !isFinite(pct)) {
            this._label.text = '—';
            this._icon.icon_name = 'battery-missing-symbolic';
            return;
        }
        const p = Math.round(pct);
        this._label.text = `${p}%`;
        this._icon.icon_name = p > 90 ? 'battery-caution-symbolic'
            : p >= 75 ? 'battery-low-symbolic' : 'battery-good-symbolic';
    }
});

// ─── 渲染器：各订阅类型 → 行描述数组 ─────────────────────────────────────────
// 行类型：bar{label,pct,text,sub} / text{label,value} / header{text} /
//         note{text} / error{text}

const barRow = (label, used, limit, resetIso) => ({
    kind: 'bar',
    label,
    pct: limPct(used, limit) ?? 0,
    text: `${fmtF(used)} / ${fmtF(limit)}`,
    sub: fmtReset(resetIso),
});

const pctBarRow = (label, pct, text, resetIso) => ({
    kind: 'bar', label, pct: pct ?? 0, text: text ?? `${(pct ?? 0).toFixed(1)}%`,
    sub: fmtReset(resetIso),
});

const RENDERERS = {
    kimi(d) {
        const rows = [];
        const meta = [d.sub_type, d.membership_level].filter(Boolean).join(' · ');
        if (meta) rows.push({kind: 'note', text: meta});
        if (d.weekly_limit > 0) rows.push(barRow('周额度', d.weekly_used, d.weekly_limit, d.weekly_reset_time));
        if (d.rp5h_limit > 0) rows.push(barRow('5 小时', d.rp5h_used, d.rp5h_limit, d.rp5h_reset_time));
        rows.push({kind: 'text', label: '总额度剩余', value: `${fmtC(d.total_remaining)} / ${fmtC(d.total_limit)}`});
        if (d.parallel_limit > 0) rows.push({kind: 'text', label: '并发上限', value: `${d.parallel_limit}`});
        return rows;
    },

    opencode(d) {
        return (d.entries || []).map(e => ({
            kind: 'bar',
            label: e.usage_type,
            pct: e.percentage ?? 0,
            text: `${e.percentage ?? 0}%`,
            sub: e.resets_in ? `${e.resets_in}后重置` : null,
        }));
    },

    xiaomi(d) {
        const rows = [];
        const exp = d.expired ? '（已过期）' : '';
        const end = d.current_period_end ? ` · 到期 ${fmtDate(d.current_period_end)}` : '';
        rows.push({kind: 'note', text: `${d.plan_name}${exp}${end}`});
        rows.push(pctBarRow('本月', d.month_percent));
        (d.entries || []).forEach(e =>
            rows.push(barRow(e.name, e.used, e.limit, null)));
        return rows;
    },

    commandcode(d) {
        const rows = [];
        const status = d.cancel_at_period_end ? `${d.subscription_status}（将取消）` : d.subscription_status;
        rows.push({kind: 'note', text: `${d.plan_name} · ${status}`});
        if (d.monthly_credits_total > 0)
            rows.push(barRow('月度积分', d.monthly_credits_used, d.monthly_credits_total, d.current_period_end));
        if (d.five_hour)
            rows.push(barRow('5 小时窗口', d.five_hour.used, d.five_hour.cap, d.five_hour.reset_at));
        if (d.weekly)
            rows.push(barRow('每周窗口', d.weekly.used, d.weekly.cap, d.weekly.reset_at));
        if (d.purchased_credits > 0)
            rows.push({kind: 'text', label: '购买积分', value: fmtF(d.purchased_credits)});
        return rows;
    },

    ollama(d) {
        const rows = [];
        rows.push({kind: 'note', text: `${d.plan_name}${d.price ? ' · ' + d.price : ''}`});
        (d.usage_entries || []).forEach(e =>
            rows.push(pctBarRow(e.usage_type, e.percentage, null, e.reset_time)));
        if (d.renews_on) rows.push({kind: 'text', label: '续期日', value: d.renews_on});
        if (d.estimated_cost_cny != null)
            rows.push({kind: 'text', label: '本周估算', value: fmtMoney(d.estimated_cost_cny)});
        return rows;
    },

    meituan(d) {
        const rows = [];
        (d.packs || []).forEach(p =>
            rows.push({
                kind: 'bar',
                label: p.package_name,
                pct: p.usage_percent ?? 0,
                text: `${fmtC(p.remain_token_amount)} / ${fmtC(p.total_token_amount)}`,
                sub: `${p.status_text} · ${p.valid_end_date_text}`,
            }));
        rows.push({kind: 'text', label: '近 7 天用量', value: fmtC(d.recent_7d_tokens)});
        return rows;
    },

    fenno(d) {
        const rows = [];
        (d.subscriptions || []).forEach(s => {
            rows.push({kind: 'header',
                text: `${s.group?.name || 'Fenno'} · ${s.status}${s.expires_at ? ' · 到期 ' + fmtDate(s.expires_at) : ''}`});
            const wins = [
                ['日额度', s.daily_usage_usd, s.group?.daily_limit_usd],
                ['周额度', s.weekly_usage_usd, s.group?.weekly_limit_usd],
                ['月额度', s.monthly_usage_usd, s.group?.monthly_limit_usd],
            ];
            wins.forEach(([label, used, limit]) => {
                if (limit && limit > 0)
                    rows.push({kind: 'bar', label, pct: limPct(used, limit) ?? 0,
                        text: `${fmtMoney(used, '$')} / ${fmtMoney(limit, '$')}`});
            });
        });
        return rows;
    },

    grok(d) {
        const rows = [];
        rows.push(pctBarRow('周额度', d.weekly_usage_percent, null, d.weekly_reset_at));
        (d.weekly_breakdown || []).forEach(b =>
            rows.push(pctBarRow(b.product, b.usage_percent)));
        rows.push({kind: 'text', label: '累计 tokens', value: fmtC(d.total_tokens)});
        if (d.estimated_cost_cny != null)
            rows.push({kind: 'text', label: '估算成本', value: fmtMoney(d.estimated_cost_cny)});
        return rows;
    },

    dimagent(d) {
        const rows = [];
        const interval = d.billing_interval === 'month' ? '月' : d.billing_interval;
        rows.push({kind: 'note', text: `${d.plan_name} · ${fmtMoney(d.price_cny)}/${interval}`});
        rows.push({
            kind: 'bar',
            label: '本期额度',
            pct: limPct(d.used_units, d.total_units) ?? 0,
            text: `${fmtF(d.used_units)} / ${fmtF(d.total_units)}`,
            sub: fmtReset(d.period_end),
        });
        if (d.estimated_remaining_calls != null)
            rows.push({kind: 'text', label: '剩余调用 ≈', value: `${fmtC(d.estimated_remaining_calls)} 次`});
        (d.feature_meters || []).forEach(m => {
            if (!m.unlimited && m.allowance > 0)
                rows.push(barRow(m.feature_key, m.used, m.allowance, m.period_end));
        });
        if (d.recent_30d)
            rows.push({kind: 'text', label: '近 30 天',
                value: `${fmtC(d.recent_30d.calls)} 次 · ${fmtC(d.recent_30d.total_tokens)} tokens`});
        return rows;
    },

    xunfei(accounts) {
        const rows = [];
        (accounts || []).forEach(acc => {
            rows.push({kind: 'header', text: acc.label || '讯飞账号'});
            if (!acc.available) {
                rows.push({kind: 'error', text: acc.error || '获取失败'});
                return;
            }
            (acc.data || []).forEach(p => {
                const u = p.usage || {};
                rows.push({
                    kind: 'bar',
                    label: p.plan_name || '套餐',
                    pct: limPct(u.package_used, u.package_limit) ?? 0,
                    text: `${fmtC(u.package_used)} / ${fmtC(u.package_limit)}`,
                    sub: p.expires_at ? `到期 ${fmtDate(p.expires_at)}` : null,
                });
                if (u.rp5h_limit > 0)
                    rows.push(barRow('5 小时', u.rp5h_used, u.rp5h_limit, null));
                if (u.rpw_limit > 0)
                    rows.push(barRow('每周', u.rpw_used, u.rpw_limit, null));
                const bal = (p.balance?.cash || 0) + (p.balance?.virtual_balance || 0);
                rows.push({kind: 'text', label: '余额', value: fmtMoney(bal)});
            });
        });
        return rows;
    },

    ainaiba(d) {
        const rows = [];
        rows.push({kind: 'text', label: '余额', value: fmtMoney(d.balance, '$')});
        rows.push({kind: 'text', label: '已用', value: `${fmtMoney(d.credit_used, '$')} / ${fmtMoney(d.credit_total, '$')}`});
        rows.push({kind: 'text', label: '最早到期', value: fmtDate(d.expires_at)});
        (d.cards || []).forEach(c =>
            rows.push({kind: 'text', label: `到账卡 ${fmtMoney(c.amount, '$')}`,
                value: `余 ${fmtMoney(c.balance, '$')} · ${fmtDate(c.expires_at)}`}));
        return rows;
    },
};

// 各类型 → 摘要用量百分比（供指示器取最紧张值）

const SUMMARY_PCT = {
    kimi: d => maxOf([
        limPct(d.weekly_used, d.weekly_limit),
        limPct(d.rp5h_used, d.rp5h_limit),
    ]),
    opencode: d => maxOf((d.entries || []).map(e => e.percentage)),
    xiaomi: d => maxOf([d.month_percent, ...(d.entries || []).map(e => e.percent)]),
    commandcode: d => maxOf([
        limPct(d.monthly_credits_used, d.monthly_credits_total),
        d.five_hour ? limPct(d.five_hour.used, d.five_hour.cap) : null,
        d.weekly ? limPct(d.weekly.used, d.weekly.cap) : null,
    ]),
    ollama: d => maxOf((d.usage_entries || []).map(e => e.percentage)),
    meituan: d => maxOf((d.packs || []).map(p => p.usage_percent)),
    fenno: d => maxOf((d.subscriptions || []).flatMap(s => [
        limPct(s.daily_usage_usd, s.group?.daily_limit_usd),
        limPct(s.weekly_usage_usd, s.group?.weekly_limit_usd),
        limPct(s.monthly_usage_usd, s.group?.monthly_limit_usd),
    ])),
    grok: d => d.weekly_usage_percent ?? null,
    dimagent: d => limPct(d.used_units, d.total_units),
    xunfei: accounts => maxOf((accounts || []).flatMap(a =>
        (a.data || []).map(p => limPct(p.usage?.package_used, p.usage?.package_limit)))),
    ainaiba: () => null,
};

// ─── 主扩展类 ────────────────────────────────────────────────────────────────

export default class QuotaWidgetExtension extends Extension {
    enable() {
        this._settings = this.getSettings();
        this._session = new Soup.Session({
            timeout: 15,
            user_agent: 'token-stats-quota-widget/1.0',
        });
        this._data = null;
        this._lastRefresh = 0;
        this._activeTab = null;
        this._refreshing = false;
        this._timeoutId = 0;
        this._signalIds = [];

        this._indicator = new QuotaIndicator();
        Main.panel.addToStatusArea('quota-widget', this._indicator, 0, 'right');
        this._buildMenuContent();

        this._signalIds.push(this._indicator.menu.connect('open-state-changed',
            (menu, open) => {
                if (open && Date.now() - this._lastRefresh > 30_000)
                    this._refresh();
            }));
        for (const key of ['quota-url', 'refresh-interval', 'hidden-cards'])
            this._signalIds.push(this._settings.connect(`changed::${key}`, () => this._onSettingsChanged(key)));

        this._startLoop();
        this._refresh();
    }

    disable() {
        this._stopLoop();
        if (this._signalIds) {
            this._signalIds.forEach(id => {
                try {
                    this._settings.disconnect(id);
                } catch {}
            });
            this._signalIds = [];
        }
        if (this._session) {
            try {
                this._session.abort();
            } catch {}
            this._session = null;
        }
        if (this._indicator) {
            this._indicator.destroy();
            this._indicator = null;
        }
        this._settings = null;
        this._data = null;
    }

    // ── 菜单 UI ──

    _buildMenuContent() {
        const mainBox = new St.BoxLayout({vertical: true, style_class: 'qw-root'});
        mainBox.style = 'width: 380px;';

        // 头部：标题 + 更新时间 + 刷新 + 打开仪表盘
        const head = new St.BoxLayout({vertical: false, style_class: 'qw-head'});
        head.add_child(new St.Label({text: '订阅配额', style_class: 'qw-head-title', x_expand: true, y_align: Clutter.ActorAlign.CENTER}));
        this._updLabel = new St.Label({text: '', style_class: 'qw-upd', y_align: Clutter.ActorAlign.CENTER});
        head.add_child(this._updLabel);
        const refreshBtn = new St.Button({
            style_class: 'qw-icon-btn button',
            child: new St.Icon({icon_name: 'view-refresh-symbolic', icon_size: 14}),
        });
        refreshBtn.connect('clicked', () => this._refresh());
        head.add_child(refreshBtn);
        const dashBtn = new St.Button({
            style_class: 'qw-icon-btn button',
            child: new St.Icon({icon_name: 'web-browser-symbolic', icon_size: 14}),
        });
        dashBtn.connect('clicked', () => {
            const base = this._settings.get_string('quota-url').replace(/\/+$/, '');
            try {
                Gio.AppInfo.launch_default_for_uri(`${base}/`, null);
            } catch (e) {
                log(`quota-widget: 无法打开仪表盘: ${e}`);
            }
        });
        head.add_child(dashBtn);
        mainBox.add_child(head);

        // Tab 行
        const tabsScroll = new St.ScrollView({
            style_class: 'qw-tabs-scroll',
            hscrollbar_policy: St.PolicyType.AUTOMATIC,
            vscrollbar_policy: St.PolicyType.NEVER,
        });
        this._tabsBox = new St.BoxLayout({vertical: false, style_class: 'qw-tabs'});
        if (tabsScroll.set_child)
            tabsScroll.set_child(this._tabsBox);
        else
            tabsScroll.add_child(this._tabsBox);
        mainBox.add_child(tabsScroll);

        // 内容区（可纵向滚动）
        const contentScroll = new St.ScrollView({
            style_class: 'qw-content-scroll vfade',
            hscrollbar_policy: St.PolicyType.NEVER,
            vscrollbar_policy: St.PolicyType.AUTOMATIC,
        });
        contentScroll.style = 'max-height: 440px;';
        this._contentBox = new St.BoxLayout({vertical: true, style_class: 'qw-content'});
        if (contentScroll.set_child)
            contentScroll.set_child(this._contentBox);
        else
            contentScroll.add_child(this._contentBox);
        mainBox.add_child(contentScroll);

        this._mainBox = mainBox;
        const item = new PopupMenu.PopupBaseMenuItem({reactive: false, can_focus: false});
        item.add_child(mainBox);
        this._indicator.menu.addMenuItem(item);

        this._rebuildTabs();
    }

    _rebuildTabs() {
        const hidden = this._settings.get_strv('hidden-cards');
        const visible = CARD_DEFS.filter(c => !hidden.includes(c.id));
        if (this._activeTab !== '__hidden' && !visible.some(c => c.id === this._activeTab))
            this._activeTab = visible.length ? visible[0].id : '__hidden';

        this._tabsBox.remove_all_children();
        const mkTab = (id, name) => {
            const btn = new St.Button({
                label: name,
                style_class: 'qw-tab' + (id === this._activeTab ? ' qw-tab-active' : ''),
            });
            btn.connect('clicked', () => {
                if (this._activeTab !== id) {
                    this._activeTab = id;
                    this._rebuildTabs();
                }
            });
            this._tabsBox.add_child(btn);
        };
        visible.forEach(c => mkTab(c.id, c.name));
        mkTab('__hidden', '管理');
        this._renderTab();
    }

    _renderTab() {
        this._contentBox.remove_all_children();
        if (this._activeTab === '__hidden') {
            this._buildHiddenManager(this._contentBox);
            return;
        }
        const card = CARD_DEFS.find(c => c.id === this._activeTab);
        if (!card)
            return;
        this._contentBox.add_child(this._buildCard(card));
    }

    _buildCard(card) {
        const status = this._getCardStatus(card.id);
        const box = new St.BoxLayout({vertical: true, style_class: 'qw-card'});

        const head = new St.BoxLayout({vertical: false, style_class: 'qw-card-head'});
        head.add_child(new St.Label({text: card.name, style_class: 'qw-card-title', x_expand: true, y_align: Clutter.ActorAlign.CENTER}));
        const hideBtn = new St.Button({
            style_class: 'qw-hide-btn',
            child: new St.Icon({icon_name: 'window-close-symbolic', icon_size: 12}),
        });
        hideBtn.connect('clicked', () => this._hideCard(card.id));
        head.add_child(hideBtn);
        box.add_child(head);

        if (!status.available || status.data == null) {
            box.add_child(this._makeNote(`获取失败${status.error ? `：${status.error}` : ''}`, 'qw-error'));
        } else {
            try {
                this._appendRows(box, RENDERERS[card.type](status.data));
            } catch (e) {
                box.add_child(this._makeNote(`渲染出错: ${e.message || e}`, 'qw-error'));
            }
        }
        if (this._lastRefresh > 0)
            box.add_child(new St.Label({
                text: `更新于 ${GLib.DateTime.new_from_unix_local(this._lastRefresh / 1000).format('%H:%M:%S')}`,
                style_class: 'qw-foot',
            }));
        return box;
    }

    _appendRows(container, rows) {
        for (const row of rows) {
            switch (row.kind) {
            case 'header':
                container.add_child(new St.Label({text: row.text, style_class: 'qw-section'}));
                break;
            case 'note':
                container.add_child(this._makeNote(row.text));
                break;
            case 'error':
                container.add_child(this._makeNote(row.text, 'qw-error'));
                break;
            case 'text': {
                const line = new St.BoxLayout({vertical: false, style_class: 'qw-row'});
                line.add_child(new St.Label({text: row.label, style_class: 'qw-label', x_expand: true}));
                line.add_child(new St.Label({text: row.value, style_class: 'qw-value'}));
                container.add_child(line);
                break;
            }
            case 'bar': {
                const line = new St.BoxLayout({vertical: false, style_class: 'qw-row'});
                line.add_child(new St.Label({text: row.label, style_class: 'qw-label', x_expand: true}));
                if (row.text)
                    line.add_child(new St.Label({text: row.text, style_class: 'qw-value'}));
                container.add_child(line);
                container.add_child(new QProgressBar({pct: row.pct, level: pctLevel(row.pct)}));
                if (row.sub)
                    container.add_child(new St.Label({text: row.sub, style_class: 'qw-sub'}));
                break;
            }
            }
        }
    }

    _makeNote(text, extraClass = '') {
        return new St.Label({text, style_class: `qw-sub qw-note${extraClass ? ' ' + extraClass : ''}`});
    }

    _buildHiddenManager(box) {
        const hidden = this._settings.get_strv('hidden-cards');
        if (!hidden.length) {
            box.add_child(this._makeNote('无隐藏订阅。点击各卡片右上角 ✕ 可隐藏过时订阅，偏好会自动保存。'));
            return;
        }
        hidden.forEach(id => {
            const def = CARD_DEFS.find(c => c.id === id);
            const row = new St.BoxLayout({vertical: false, style_class: 'qw-row'});
            row.add_child(new St.Label({text: def ? def.name : id, style_class: 'qw-label', x_expand: true}));
            const btn = new St.Button({label: '恢复', style_class: 'qw-restore-btn button'});
            btn.connect('clicked', () => this._unhideCard(id));
            row.add_child(btn);
            box.add_child(row);
        });
    }

    // ── 隐藏订阅（GSettings 持久化）──

    _hideCard(id) {
        const hidden = this._settings.get_strv('hidden-cards');
        if (!hidden.includes(id))
            this._settings.set_strv('hidden-cards', [...hidden, id]);
    }

    _unhideCard(id) {
        this._settings.set_strv('hidden-cards',
            this._settings.get_strv('hidden-cards').filter(h => h !== id));
    }

    _onSettingsChanged(key) {
        if (key === 'refresh-interval')
            this._startLoop();
        else if (key === 'quota-url')
            this._refresh();
        else if (key === 'hidden-cards') {
            this._rebuildTabs();
            this._updateIndicator();
        }
    }

    // ── 数据 ──

    _getCardStatus(id) {
        const q = this._data?.quota || {};
        if (id === 'xunfei') {
            const x = this._data?.xunfei;
            if (!x || !Array.isArray(x.accounts))
                return {available: false, data: null, error: '未加载'};
            const ok = x.accounts.some(acc => acc.available);
            return {available: ok, data: x.accounts,
                error: ok ? null : (x.accounts[0]?.error || '无可用账号')};
        }
        if (id === 'ainaiba') {
            const a = this._data?.ainaiba;
            return a || {available: false, data: null, error: '未加载'};
        }
        const def = CARD_DEFS.find(c => c.id === id);
        return (def && q[def.key]) || {available: false, data: null, error: '未加载'};
    }

    _updateIndicator() {
        const hidden = this._settings.get_strv('hidden-cards');
        let worst = null;
        for (const card of CARD_DEFS) {
            if (hidden.includes(card.id))
                continue;
            const st = this._getCardStatus(card.id);
            if (!st.available || st.data == null)
                continue;
            const fn = SUMMARY_PCT[card.type];
            if (!fn)
                continue;
            try {
                const p = fn(st.data);
                if (p != null && isFinite(p))
                    worst = worst == null ? p : Math.max(worst, p);
            } catch {}
        }
        this._indicator.setSummary(worst);
    }

    async _refresh() {
        if (this._refreshing)
            return;
        this._refreshing = true;
        try {
            const base = this._settings.get_string('quota-url').replace(/\/+$/, '');
            const [q, x, a] = await Promise.allSettled([
                this._fetchJson(`${base}/api/quota`),
                this._fetchJson(`${base}/api/xunfei`),
                this._fetchJson(`${base}/api/ainaiba-credit`),
            ]);
            this._data = {
                quota: q.status === 'fulfilled' ? q.value : null,
                xunfei: x.status === 'fulfilled' ? x.value : null,
                ainaiba: a.status === 'fulfilled' ? a.value : null,
            };
            this._lastRefresh = Date.now();
            this._updLabel.text = GLib.DateTime.new_from_unix_local(this._lastRefresh / 1000).format('%H:%M');
            this._updateIndicator();
            if (this._indicator?.menu.isOpen)
                this._renderTab();
        } finally {
            this._refreshing = false;
        }
    }

    _fetchJson(url) {
        return new Promise((resolve, reject) => {
            let msg;
            try {
                msg = Soup.Message.new('GET', url);
            } catch (e) {
                reject(e);
                return;
            }
            if (!msg) {
                reject(new Error(`无效 URL: ${url}`));
                return;
            }
            this._session.send_and_read_async(msg, GLib.PRIORITY_DEFAULT, null,
                (session, result) => {
                    try {
                        const bytes = session.send_and_read_finish(result);
                        const code = msg.get_status_code();
                        if (code !== 200) {
                            reject(new Error(`HTTP ${code}`));
                            return;
                        }
                        resolve(JSON.parse(new TextDecoder().decode(bytes.get_data())));
                    } catch (e) {
                        reject(e);
                    }
                });
        });
    }

    _startLoop() {
        this._stopLoop();
        const interval = Math.max(10, this._settings.get_int('refresh-interval'));
        this._timeoutId = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, interval, () => {
            this._refresh();
            return GLib.SOURCE_CONTINUE;
        });
    }

    _stopLoop() {
        if (this._timeoutId) {
            GLib.source_remove(this._timeoutId);
            this._timeoutId = 0;
        }
    }
}
