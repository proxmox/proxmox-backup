Ext.define('PBS.window.GCJobEdit', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsGCJobEdit',
    mixins: ['Proxmox.Mixin.CBind'],

    userid: undefined,
    onlineHelp: 'maintenance_gc',
    isAdd: false,

    subject: gettext('Garbage Collect Schedule'),

    cbindData: function(initial) {
        let me = this;

        me.datastore = encodeURIComponent(me.datastore);
	me.url = `/api2/extjs/config/datastore/${me.datastore}`;
        me.method = 'PUT';
        me.autoLoad = true;
	return {};
    },

    items: {
	xtype: 'pbsCalendarEvent',
	name: 'gc-schedule',
	fieldLabel: gettext("GC Schedule"),
	emptyText: gettext(Proxmox.Utils.NoneText + " (disabled)"),
    },
});
