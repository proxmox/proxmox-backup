Ext.define('PBS.window.SafeDatastoreDestroy', {
    extend: 'Proxmox.window.SafeDestroy',
    xtype: 'pbsDatastoreSafeDestroy',
    mixins: ['Proxmox.Mixin.CBind'],

    cbind: {
	url: `/config/datastore/{datastore}`,
	item: get => ({ id: get('datastore') }),
    },
    viewModel: {
	data: {
	    'destroyData': 0,
	    'keepJobConfigs': 0,
	},
	formulas: {
	    destroyNote: get => get('destroyData')
		? gettext('All backup snapshots and their data will be permanently destroyed!')
		: gettext('Configuration change only, no data will be deleted.'),
	    destroyNoteCls: get => get('destroyData') ? 'pmx-hint' : '',
	},
    },

    autoShow: true,
    taskName: 'delete-datastore',

    apiCallDone: function(success) {
	if (success) {
	    let navtree = Ext.ComponentQuery.query('navigationtree')[0];
	    navtree.rstore.load();
	    let mainview = Ext.ComponentQuery.query('mainview')[0];
	    mainview.getController().redirectTo('pbsDataStores');
	}
    },

    getParams: function() {
	let viewModel = this.getViewModel();

	let params = {
	    'destroy-data': viewModel.get('destroyData'),
	    'keep-job-configs': viewModel.get('keepJobConfigs'),
	};
	return `?${Ext.Object.toQueryString(params)}`;
    },
    additionalItems: [{
	xtype: 'proxmoxcheckbox',
	name: 'destroy-data',
	boxLabel: gettext("Destroy all data (dangerous!)"),
	defaultValue: false,
	bind: {
	    value: '{destroyData}',
	},
    }, {
	xtype: 'proxmoxcheckbox',
	name: 'keep-job-configs',
	boxLabel: gettext("Keep configured jobs and permissions"),
	defaultValue: false,
	bind: {
	    value: '{keepJobConfigs}',
	},
    }, {
	xtype: 'component',
	reference: 'noteCmp',
	bind: {
	    html: '{destroyNote}',
	    userCls: '{destroyNoteCls}',
	},
    }],
});

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
	    Ext.create('PBS.window.SafeDatastoreDestroy', {
		datastore: me.getView().datastore,
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
		for (const k of ['Verify', 'Sync', 'GC', 'Prune']) {
		    let fallback = k === 'Prune' ? 'Error' : 'Always';
		    let v = Ext.String.capitalize(notify[k.toLowerCase()]) || fallback;
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
	'tuning': {
	    required: true,
	    header: gettext('Tuning Options'),
	    renderer: v => PBS.Utils.render_tuning_options(PBS.Utils.parsePropertyString(v)),
	    editor: {
		xtype: 'proxmoxWindowEdit',
		title: gettext('Tuning Options'),
		onlineHelp: 'datastore_tuning_options',
		width: 350,
		items: {
		    xtype: 'inputpanel',
		    onGetValues: function(values) {
			if (!Ext.isArray(values.delete ?? [])) {
			    values.delete = [values.delete];
			}
			for (const k of values.delete ?? []) {
			    delete values[k];
			}
			delete values.delete;
			let tuning = PBS.Utils.printPropertyString(values);
			if (!tuning) {
			    return { 'delete': 'tuning' };
			}
			return { tuning };
		    },
		    onSetValues: values => PBS.Utils.parsePropertyString(values?.tuning),
		    items: [
			{
			    xtype: 'proxmoxKVComboBox',
			    name: 'chunk-order',
			    fieldLabel: gettext('Chunk Order'),
			    comboItems: Object.entries(PBS.Utils.tuningOptions['chunk-order']),
			    deleteEmpty: true,
			    value: '__default__',
			},
			{
			    xtype: 'proxmoxKVComboBox',
			    name: 'sync-level',
			    fieldLabel: gettext('Sync Level'),
			    comboItems: Object.entries(PBS.Utils.tuningOptions['sync-level']),
			    deleteEmpty: true,
			    value: '__default__',
			},
		    ],
		},
	    },
	},
    },
});
