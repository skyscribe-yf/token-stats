// quota-widget@token-stats 偏好设置窗口

import Adw from 'gi://Adw';
import Gtk from 'gi://Gtk';

import {ExtensionPreferences} from 'resource:///org/gnome/shell/extensions/extensionPreferences.js';

// 与 extension.js 中 CARD_DEFS 保持一致（仅 id + name）
const CARD_DEFS = [
    {id: 'kimi', name: 'Kimi'},
    {id: 'kimi-ex', name: 'Kimi EX'},
    {id: 'opencode-go', name: 'OpenCode Go'},
    {id: 'opencode-go-ex', name: 'OpenCode EX'},
    {id: 'xiaomi-mimo', name: 'MiMo'},
    {id: 'commandcode', name: 'Command Code'},
    {id: 'commandcode-ex', name: 'Cmd Code EX'},
    {id: 'ollama', name: 'Ollama'},
    {id: 'meituan', name: 'LongCat'},
    {id: 'fenno', name: 'Fenno'},
    {id: 'fenno-ex', name: 'Fenno EX'},
    {id: 'grok', name: 'Grok'},
    {id: 'dimagent', name: 'DimAgent'},
    {id: 'xunfei', name: '讯飞'},
    {id: 'ainaiba', name: 'Ainaba'},
];

export default class QuotaWidgetPreferences extends ExtensionPreferences {
    fillPreferencesWindow(window) {
        const settings = this.getSettings();

        const page = new Adw.PreferencesPage();
        window.add(page);

        // 数据源
        const srcGroup = new Adw.PreferencesGroup({title: '数据源'});
        page.add(srcGroup);

        const urlRow = new Adw.EntryRow({title: 'Token Stats 后端地址', show_apply_button: true});
        urlRow.text = settings.get_string('quota-url');
        urlRow.connect('apply', () => {
            const v = urlRow.text.trim().replace(/\/+$/, '');
            settings.set_string('quota-url', v || 'http://127.0.0.1:3000');
        });
        srcGroup.add(urlRow);

        const intervalRow = new Adw.SpinRow.new_with_range(10, 3600, 10);
        intervalRow.title = '自动刷新间隔（秒）';
        intervalRow.value = settings.get_int('refresh-interval');
        intervalRow.connect('changed', () => settings.set_int('refresh-interval', intervalRow.value));
        srcGroup.add(intervalRow);

        // 隐藏的订阅
        const hiddenGroup = new Adw.PreferencesGroup({
            title: '隐藏的订阅',
            description: '在面板弹窗中点击卡片右上角 ✕ 可隐藏订阅；点击「恢复」重新显示。',
        });
        page.add(hiddenGroup);

        const hidden = settings.get_strv('hidden-cards');
        if (!hidden.length) {
            hiddenGroup.add(new Adw.ActionRow({title: '无隐藏订阅'}));
        } else {
            hidden.forEach(id => {
                const def = CARD_DEFS.find(c => c.id === id);
                const row = new Adw.ActionRow({title: def ? def.name : id});
                const btn = new Gtk.Button({label: '恢复', valign: Gtk.Align.CENTER});
                btn.connect('clicked', () => {
                    settings.set_strv('hidden-cards',
                        settings.get_strv('hidden-cards').filter(h => h !== id));
                    row.set_sensitive(false);
                    btn.set_sensitive(false);
                });
                row.add_suffix(btn);
                hiddenGroup.add(row);
            });
        }
    }
}
