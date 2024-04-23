Ext.define('PBS.config.NotificationConfigView', {
    extend: 'Proxmox.panel.NotificationConfigView',
    alias: ['widget.pbsNotificationConfigView'],
    mixins: ['Proxmox.Mixin.CBind'],

    cbindData: function(_initialConfig) {
        return {
            baseUrl: '/config/notifications',
        };
    },
});
