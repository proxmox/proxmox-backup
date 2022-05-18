Ext.define('PBS.data.PermissionPathsStore', {
    extend: 'Ext.data.Store',
    alias: 'store.pbsPermissionPaths',
    fields: ['value'],
    autoLoad: false,
    data: [
	{ 'value': '/' },
	{ 'value': '/access' },
	{ 'value': '/access/acl' },
	{ 'value': '/access/users' },
	{ 'value': '/access/domains' },
	{ 'value': '/access/openid' },
	{ 'value': '/datastore' },
	{ 'value': '/remote' },
	{ 'value': '/system' },
	{ 'value': '/system/certificates' },
	{ 'value': '/system/disks' },
	{ 'value': '/system/log' },
	{ 'value': '/system/network' },
	{ 'value': '/system/network/dns' },
	{ 'value': '/system/network/interfaces' },
	{ 'value': '/system/services' },
	{ 'value': '/system/status' },
	{ 'value': '/system/tasks' },
	{ 'value': '/system/time' },
	{ 'value': '/tape' },
	{ 'value': '/tape/device' },
	{ 'value': '/tape/pool' },
	{ 'value': '/tape/job' },
    ],

    constructor: function(config) {
	let me = this;

	config = config || {};
	me.callParent([config]);

	// TODO: this is but a HACK until we have some sort of resource storage like PVE
	let datastores = Ext.data.StoreManager.lookup('pbs-datastore-list');

	if (datastores) {
	    let donePaths = {};
	    me.suspendEvents();
	    datastores.each(function(record) {
		let path = `/datastore/${record.data.store}`;
		if (path !== undefined && !donePaths[path]) {
		    me.add({ value: path });
		    donePaths[path] = 1;
		}
	    });
	    me.resumeEvents();

	    if (me.datastore) {
		me.setDatastore(me.datastore);
	    }

	    me.fireEvent('refresh', me);
	    me.fireEvent('datachanged', me);
	}

	me.sort({
	    property: 'value',
	    direction: 'ASC',
	});
	me.initialized = true;
    },

    setDatastore: async function(datastore) {
	let me = this;
	if (!datastore) {
	    me.clearFilter();
	    return;
	}
	let url = `/api2/extjs/admin/datastore/${datastore}/namespace?max-depth=7`;
	let { result: { data: ns } } = await Proxmox.Async.api2({ url });
	// TODO: remove "old" datastore's ns paths?
	if (ns.length > 0) {
	    if (me.initialized) {
		me.suspendEvents();
	    }
	    for (const item of ns) {
		if (item.ns !== '') {
		    me.add({ value: `/datastore/${datastore}/${item.ns}` });
		}
	    }
	    if (me.initialized) {
		me.resumeEvents();
		me.fireEvent('refresh', me);
		me.fireEvent('datachanged', me);
	    }
	}
	me.filter(item => item.get('value')?.startsWith(`/datastore/${datastore}`));
    },
});

Ext.define('PBS.form.PermissionPathSelector', {
    extend: 'Ext.form.field.ComboBox',
    xtype: 'pbsPermissionPathSelector',
    mixins: ['Proxmox.Mixin.CBind'],

    config: {
	datastore: null, // set to filter by a datastore, could be also made generic path
    },

    setDatastore: function(datastore) {
	let me = this;
	if (me.datastore === datastore) {
	    return;
	}
	me.datastore = datastore;
	let store = me.getStore();
	if (!me.rendered) {
	    if (store) {
		store.datastore = datastore;
	    }
	} else {
	    store.setDatastore(datastore);
	}
    },

    valueField: 'value',
    displayField: 'value',
    cbind: {
	typeAhead: '{editable}',
    },
    anyMatch: true,
    queryMode: 'local',

    store: {
	type: 'pbsPermissionPaths',
    },
    regexText: gettext('Invalid permission path.'),
    regex: /\/((access|datastore|remote|system)\/.*)?/,
});
