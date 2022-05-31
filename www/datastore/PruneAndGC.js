Ext.define('PBS.Datastore.GCOptions', {
    extend: 'Proxmox.grid.ObjectGrid',
    alias: 'widget.pbsDatastoreGCOpts',
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
    },
});

Ext.define('PBS.Datastore.PruneAndGC', {
    extend: 'Ext.panel.Panel',
    alias: 'widget.pbsDatastorePruneAndGC',
    mixins: ['Proxmox.Mixin.CBind'],

    layout: {
	type: 'vbox',
	align: 'stretch',
	multi: true,
    },
    defaults: {
	collapsible: false,
	margin: '7 10 3 10',
    },
    items: [
	{
	    xtype: 'pbsDatastoreGCOpts',
	    title: gettext('Garbage Collection'),
	    itemId: 'datastore-gc',
	    nodename: 'localhost',
	    cbind: {
		datastore: '{datastore}',
	    },
	},
	{
	    xtype: 'pbsPruneJobView',
	    nodename: 'localhost',
	    itemId: 'datastore-prune-jobs',
	    flex: 1,
	    minHeight: 200,
	    cbind: {
		datastore: '{datastore}',
	    },
	},
    ],
    initComponent: function() {
	let me = this;

	let subPanelIds = me.items.map(el => el.itemId);

	me.callParent();

	for (const itemId of subPanelIds) {
	    let component = me.getComponent(itemId);
	    component.relayEvents(me, ['activate', 'deactivate', 'destroy']);
	}
    },
});
