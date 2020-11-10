Ext.define('PBS.DataStorePruneAndGC', {
    extend: 'Proxmox.grid.ObjectGrid',
    alias: 'widget.pbsDataStorePruneAndGC',
    mixins: ['Proxmox.Mixin.CBind'],

    onlineHelp: 'maintenance_pruning',

    cbindData: function(initial) {
	let me = this;

	me.datastore = encodeURIComponent(me.datastore);
	me.url = `/api2/json/config/datastore/${me.datastore}`;
	me.editorConfig = {
	    url: `/api2/extjs/config/datastore/${me.datastore}`,
	};
	return {};
    },

    controller: {
	xclass: 'Ext.app.ViewController',

	edit: function() { this.getView().run_editor(); },

	garbageCollect: function() {
	    let me = this;
	    let view = me.getView();
	    Proxmox.Utils.API2Request({
		url: `/admin/datastore/${view.datastore}/gc`,
		method: 'POST',
		failure: function(response) {
		    Ext.Msg.alert(gettext('Error'), response.htmlStatus);
		},
		success: function(response, options) {
		    Ext.create('Proxmox.window.TaskViewer', {
			upid: response.result.data,
		    }).show();
		},
	    });
	},
    },

    tbar: [
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Edit'),
	    disabled: true,
	    handler: 'edit',
	},
	'-',
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Start Garbage Collection'),
	    selModel: null,
	    handler: 'garbageCollect',
	},
    ],

    listeners: {
	activate: function() { this.rstore.startUpdate(); },
	destroy: function() { this.rstore.stopUpdate(); },
	deactivate: function() { this.rstore.stopUpdate(); },
	itemdblclick: 'edit',
    },

    rows: {
	"gc-schedule": {
	    required: true,
	    defaultValue: Proxmox.Utils.NoneText,
	    header: gettext('Garbage Collection Schedule'),
	    editor: {
		xtype: 'proxmoxWindowEdit',
		title: gettext('GC Schedule'),
		onlineHelp: 'maintenance_gc',
		items: {
		    xtype: 'pbsCalendarEvent',
		    name: 'gc-schedule',
		    fieldLabel: gettext("GC Schedule"),
		    emptyText: Proxmox.Utils.noneText,
		    deleteEmpty: true,
		},
	    },
	},
	"prune-schedule": {
	    required: true,
	    defaultValue: Proxmox.Utils.NoneText,
	    header: gettext('Prune Schedule'),
	    editor: {
		xtype: 'proxmoxWindowEdit',
		title: gettext('Prune Schedule'),
		onlineHelp: 'maintenance_pruning',
		items: {
		    xtype: 'pbsCalendarEvent',
		    name: 'prune-schedule',
		    fieldLabel: gettext("Prune Schedule"),
		    emptyText: Proxmox.Utils.noneText,
		    deleteEmpty: true,
		},
	    },
	},
	"keep-last": {
	    required: true,
	    header: gettext('Keep Last'),
	    editor: {
		xtype: 'proxmoxWindowEdit',
		title: gettext('Prune Options'),
		onlineHelp: 'maintenance_pruning',
		items: {
		    xtype: 'pbsPruneInputPanel',
		    isCreate: false,
		},
	    },
	},
	"keep-hourly": {
	    required: true,
	    header: gettext('Keep Hourly'),
	    editor: {
		xtype: 'proxmoxWindowEdit',
		title: gettext('Prune Options'),
		onlineHelp: 'maintenance_pruning',
		items: {
		    xtype: 'pbsPruneInputPanel',
		},
	    },
	},
	"keep-daily": {
	    required: true,
	    header: gettext('Keep Daily'),
	    editor: {
		xtype: 'proxmoxWindowEdit',
		title: gettext('Prune Options'),
		onlineHelp: 'maintenance_pruning',
		items: {
		    xtype: 'pbsPruneInputPanel',
		},
	    },
	},
	"keep-weekly": {
	    required: true,
	    header: gettext('Keep Weekly'),
	    editor: {
		xtype: 'proxmoxWindowEdit',
		title: gettext('Prune Options'),
		onlineHelp: 'maintenance_pruning',
		items: {
		    xtype: 'pbsPruneInputPanel',
		},
	    },
	},
	"keep-monthly": {
	    required: true,
	    header: gettext('Keep Monthly'),
	    editor: {
		xtype: 'proxmoxWindowEdit',
		title: gettext('Prune Options'),
		onlineHelp: 'maintenance_pruning',
		items: {
		    xtype: 'pbsPruneInputPanel',
		},
	    },
	},
	"keep-yearly": {
	    required: true,
	    header: gettext('Keep Yearly'),
	    editor: {
		xtype: 'proxmoxWindowEdit',
		title: gettext('Prune Options'),
		onlineHelp: 'maintenance_pruning',
		items: {
		    xtype: 'pbsPruneInputPanel',
		},
	    },
	},
    },
});
