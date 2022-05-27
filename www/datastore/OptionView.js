
Ext.define('PBS.Datastore.Options', {
    extend: 'Proxmox.grid.ObjectGrid',
    xtype: 'pbsDatastoreOptionView',
    mixins: ['Proxmox.Mixin.CBind'],

    cbindData: function(initial) {
	let me = this;

	me.maintenanceActiveTasks = {
	    read: 0,
	    write: 0,
	};
	me.datastore = encodeURIComponent(me.datastore);
	me.url = `/api2/json/config/datastore/${me.datastore}`;
	me.editorConfig = {
	    url: `/api2/extjs/config/datastore/${me.datastore}`,
	    datastore: me.datastore,
	};
	return {};
    },

    controller: {
	xclass: 'Ext.app.ViewController',

	init: function(view) {
	    let me = this;

	    me.activeOperationsRstore = Ext.create('Proxmox.data.ObjectStore', {
		url: `/api2/json/admin/datastore/${view.datastore}/active-operations`,
		interval: 3000,
	    });
	    me.activeOperationsRstore.startUpdate();

	    view.mon(me.activeOperationsRstore, 'load', (store, data, success) => {
		let activeTasks = me.getView().maintenanceActiveTasks;
		activeTasks.read = data?.[0]?.data.value ?? 0;
		activeTasks.write = data?.[1]?.data.value ?? 0;
	    });
	},

	edit: function() {
	    this.getView().run_editor();
	},

	removeDatastore: function() {
	    let me = this;
	    let datastore = me.getView().datastore;
	    Ext.create('Proxmox.window.SafeDestroy', {
		url: `/config/datastore/${datastore}`,
		item: {
		    id: datastore,
		},
		note: gettext('Configuration change only, no data will be deleted.'),
		autoShow: true,
		taskName: 'delete-datastore',
		apiCallDone: (success) => {
		    let navtree = Ext.ComponentQuery.query('navigationtree')[0];
		    navtree.rstore.load();
		    let mainview = me.getView().up('mainview');
		    mainview.getController().redirectTo('pbsDataStores');
		},
	    });
	},

	stopUpdates: function() {
	    let me = this;
	    let view = me.getView();

	    view.rstore.stopUpdate();
	    me.activeOperationsRstore.stopUpdate();
	},
	startUpdates: function() {
	    let me = this;
	    let view = me.getView();

	    view.rstore.startUpdate();
	    me.activeOperationsRstore.startUpdate();
	},
    },

    tbar: [
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Edit'),
	    disabled: true,
	    handler: 'edit',
	},
	'->',
	{
	    xtype: 'proxmoxButton',
	    selModel: null,
	    iconCls: 'fa fa-trash-o',
	    text: gettext('Remove Datastore'),
	    handler: 'removeDatastore',
	},
    ],

    listeners: {
	activate: 'startUpdates',
	beforedestroy: 'stopUpdates',
	deactivate: 'stopUpdates',
	itemdblclick: 'edit',
    },

    rows: {
	"notify": {
	    required: true,
	    header: gettext('Notify'),
	    renderer: (value) => {
		let notify = PBS.Utils.parsePropertyString(value);
		let res = [];
		for (const k of ['Verify', 'Sync', 'GC']) {
		    let v = Ext.String.capitalize(notify[k.toLowerCase()]) || 'Always';
		    res.push(`${k}=${v}`);
		}
		return res.join(', ');
	    },
	    editor: {
		xtype: 'pbsNotifyOptionEdit',
	    },
	},
	"notify-user": {
	    required: true,
	    defaultValue: 'root@pam',
	    header: gettext('Notify User'),
	    editor: {
		xtype: 'pbsNotifyOptionEdit',
	    },
	},
	"verify-new": {
	    required: true,
	    header: gettext('Verify New Snapshots'),
	    defaultValue: false,
	    renderer: Proxmox.Utils.format_boolean,
	    editor: {
		xtype: 'proxmoxWindowEdit',
		title: gettext('Verify New'),
		width: 350,
		items: {
		    xtype: 'proxmoxcheckbox',
		    name: 'verify-new',
		    boxLabel: gettext("Verify new backups immediately after completion"),
		    defaultValue: false,
		    deleteDefaultValue: true,
		    deleteEmpty: true,
		},
	    },
	},
	"maintenance-mode": {
	    required: true,
	    header: gettext('Maintenance mode'),
	    renderer: function(v) {
		return PBS.Utils.renderMaintenance(v, this.maintenanceActiveTasks);
	    },
	    editor: {
		xtype: 'pbsMaintenanceOptionEdit',
	    },
	},
    },
});
