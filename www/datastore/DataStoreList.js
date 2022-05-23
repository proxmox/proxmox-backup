// Overview over all datastores
Ext.define('PBS.datastore.DataStoreList', {
    extend: 'Ext.panel.Panel',
    alias: 'widget.pbsDataStoreList',

    title: gettext('Summary'),

    scrollable: true,

    bodyPadding: 5,
    defaults: {
	xtype: 'pbsDataStoreListSummary',
	padding: 5,
    },

    datastores: {},
    tasks: {},

    updateTasks: function(taskStore, records, success) {
	let me = this;
	if (!success) {
	    return;
	}

	for (const store of Object.keys(me.tasks)) {
	    me.tasks[store] = {};
	}

	records.forEach(record => {
	    let task = record.data;
	    if (!task.worker_id) {
		return;
	    }

	    let type = task.worker_type;
	    if (type === 'syncjob') {
		type = 'sync';
	    }

	    if (type.startsWith('verif')) {
		type = 'verify';
	    }

	    if (type === 'prunejob') {
		type = 'prune';
	    }

	    let datastore = PBS.Utils.parse_datastore_worker_id(type, task.worker_id);
	    if (!datastore) {
		return;
	    }

	    if (!me.tasks[datastore]) {
		me.tasks[datastore] = {};
	    }

	    if (!me.tasks[datastore][type]) {
		me.tasks[datastore][type] = {};
	    }

	    if (me.tasks[datastore][type] && task.status) {
		let parsed = Proxmox.Utils.parse_task_status(task.status);
		if (!me.tasks[datastore][type][parsed]) {
		    me.tasks[datastore][type][parsed] = 0;
		}
		me.tasks[datastore][type][parsed]++;
	    }
	});

	for (const [store, panel] of Object.entries(me.datastores)) {
	    panel.setTasks(me.tasks[store], me.since);
	}
    },

    updateStores: function(usageStore, records, success) {
	let me = this;
	if (!success) {
	    return;
	}

	let found = {};

	records.forEach((rec) => {
	    found[rec.data.store] = true;
	    me.addSorted(rec.data);
	});

	for (const [store, panel] of Object.entries(me.datastores)) {
	    if (!found[store]) {
		me.remove(panel);
		delete me.datastores[store];
	    }
	}

	let hasDatastores = Object.keys(me.datastores).length > 0;

	me.getComponent('emptybox').setHidden(hasDatastores);
    },

    addSorted: function(data) {
	let me = this;
	let i = 1;
	let datastores = Object
	    .keys(me.datastores)
	    .sort((a, b) => a.localeCompare(b));

	for (const datastore of datastores) {
	    let result = datastore.localeCompare(data.store);
	    if (result === 0) {
		me.datastores[datastore].setStatus(data);
		return;
	    } else if (result > 0) {
		break;
	    }
	    i++;
	}

	me.datastores[data.store] = me.insert(i, {
	    datastore: data.store,
	});
	me.datastores[data.store].setStatus(data);
	me.datastores[data.store].setTasks(me.tasks[data.store], me.since);
    },

    initComponent: function() {
	let me = this;
	me.items = [
	    {
		itemId: 'emptybox',
		hidden: true,
		xtype: 'box',
		html: gettext('No Datastores configured'),
	    },
	];
	me.datastores = {};
	// todo make configurable?
	me.since = (Date.now()/1000 - 30 * 24*3600).toFixed(0);

	me.usageStore = Ext.create('Proxmox.data.UpdateStore', {
	    storeid: 'datastore-overview-usage',
	    interval: 5000,
	    proxy: {
		type: 'proxmox',
		url: '/api2/json/status/datastore-usage',
	    },
	    listeners: {
		load: {
		    fn: me.updateStores,
		    scope: me,
		},
	    },
	});

	me.taskStore = Ext.create('Proxmox.data.UpdateStore', {
	    storeid: 'datastore-overview-tasks',
	    interval: 15000,
	    model: 'proxmox-tasks',
	    proxy: {
		type: 'proxmox',
		url: '/api2/json/nodes/localhost/tasks',
		extraParams: {
		    limit: 0,
		    since: me.since,
		},
	    },
	    listeners: {
		load: {
		    fn: me.updateTasks,
		    scope: me,
		},
	    },
	});

	me.callParent();
	Proxmox.Utils.monStoreErrors(me, me.usageStore);
	Proxmox.Utils.monStoreErrors(me, me.taskStore);
	me.on('activate', function() {
	    me.usageStore.startUpdate();
	    me.taskStore.startUpdate();
	});
	me.on('destroy', function() {
	    me.usageStore.stopUpdate();
	    me.taskStore.stopUpdate();
	});
	me.on('deactivate', function() {
	    me.usageStore.stopUpdate();
	    me.taskStore.stopUpdate();
	});
    },
});

Ext.define('PBS.datastore.DataStores', {
    extend: 'Ext.tab.Panel',
    alias: 'widget.pbsDataStores',

    title: gettext('Datastores'),

    stateId: 'pbs-datastores-panel',
    stateful: true,

    stateEvents: ['tabchange'],

    applyState: function(state) {
	let me = this;
	if (state.tab !== undefined && me.rendered) {
	    me.setActiveTab(state.tab);
	} else if (state.tab) {
	    // if we are not rendered yet, defer setting the activetab
	    setTimeout(function() {
		me.setActiveTab(state.tab);
	    }, 10);
	}
    },

    getState: function() {
	let me = this;
	return {
	    tab: me.getActiveTab().getItemId(),
	};
    },

    border: false,
    defaults: {
	border: false,
    },

    tools: [PBS.Utils.get_help_tool("datastore_intro")],

    items: [
	{
	    xtype: 'pbsDataStoreList',
	    iconCls: 'fa fa-book',
	},

	{
	    iconCls: 'fa fa-refresh',
	    itemId: 'syncjobs',
	    xtype: 'pbsSyncJobView',
	},
	{
	    iconCls: 'fa fa-trash-o',
	    itemId: 'prunejobs',
	    xtype: 'pbsPruneJobView',
	},
	{
	    iconCls: 'fa fa-check-circle',
	    itemId: 'verifyjobs',
	    xtype: 'pbsVerifyJobView',
	},
	{
	    itemId: 'acl',
	    xtype: 'pbsACLView',
	    iconCls: 'fa fa-unlock',
	    aclPath: '/datastore',
	},
    ],

    initComponent: function() {
	let me = this;
	// remove invalid activeTab settings
	if (me.activeTab && !me.items.some((item) => item.itemId === me.activeTab)) {
	    delete me.activeTab;
	}
	me.callParent();
    },
});
