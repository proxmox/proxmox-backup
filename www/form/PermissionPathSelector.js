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
	{ 'value': '/datastore' },
	{ 'value': '/remote' },
	{ 'value': '/system' },
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

	    me.fireEvent('refresh', me);
	    me.fireEvent('datachanged', me);
	}

	me.sort({
	    property: 'value',
	    direction: 'ASC',
	});
    },
});

Ext.define('PBS.form.PermissionPathSelector', {
    extend: 'Ext.form.field.ComboBox',
    xtype: 'pbsPermissionPathSelector',
    mixins: ['Proxmox.Mixin.CBind'],

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
