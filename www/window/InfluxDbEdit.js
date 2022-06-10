Ext.define('PBS.window.InfluxDbHttpEdit', {
    extend: 'Proxmox.window.Edit',
    mixins: ['Proxmox.Mixin.CBind'],

    subject: 'InfluxDB (HTTP)',

    cbindData: function() {
	let me = this;
	me.isCreate = !me.serverid;
	me.serverid = me.serverid || "";
	me.url = `/api2/extjs/config/metrics/influxdb-http/${me.serverid}`;
	me.tokenEmptyText = me.isCreate ? '' : gettext('unchanged');
	me.method = me.isCreate ? 'POST' : 'PUT';
	if (!me.isCreate) {
	    me.subject = `${me.subject}: ${me.serverid}`;
	}
	return {};
    },

    items: [
	{
	    xtype: 'inputpanel',

	    column1: [
		{
		    xtype: 'pmxDisplayEditField',
		    name: 'name',
		    fieldLabel: gettext('Name'),
		    allowBlank: false,
		    cbind: {
			editable: '{isCreate}',
			value: '{serverid}',
		    },
		},
		{
		    xtype: 'proxmoxtextfield',
		    name: 'url',
		    fieldLabel: gettext('URL'),
		    allowBlank: false,
		},
	    ],

	    column2: [
		{
		    xtype: 'checkbox',
		    name: 'enable',
		    fieldLabel: gettext('Enabled'),
		    inputValue: 1,
		    uncheckedValue: 0,
		    checked: true,
		},
		{
		    xtype: 'proxmoxtextfield',
		    name: 'organization',
		    fieldLabel: gettext('Organization'),
		    emptyText: 'proxmox',
		    cbind: {
			deleteEmpty: '{!isCreate}',
		    },
		},
		{
		    xtype: 'proxmoxtextfield',
		    name: 'bucket',
		    fieldLabel: gettext('Bucket'),
		    emptyText: 'proxmox',
		    cbind: {
			deleteEmpty: '{!isCreate}',
		    },
		},
		{
		    xtype: 'proxmoxtextfield',
		    name: 'token',
		    fieldLabel: gettext('Token'),
		    allowBlank: true,
		    deleteEmpty: false,
		    submitEmpty: false,
		    cbind: {
			emptyText: '{tokenEmptyText}',
		    },
		},
	    ],

	    columnB: [
		{
		    xtype: 'proxmoxtextfield',
		    name: 'comment',
		    fieldLabel: gettext('Comment'),
		    cbind: {
			deleteEmpty: '{!isCreate}',
		    },
		},
	    ],

	    advancedColumn1: [
		{
		    xtype: 'proxmoxintegerfield',
		    name: 'max-body-size',
		    fieldLabel: gettext('Batch Size (b)'),
		    minValue: 1,
		    emptyText: '25000000',
		    submitEmpty: false,
		    cbind: {
			deleteEmpty: '{!isCreate}',
		    },
		},
	    ],
	},
    ],
});

Ext.define('PBS.window.InfluxDbUdpEdit', {
    extend: 'Proxmox.window.Edit',
    mixins: ['Proxmox.Mixin.CBind'],

    subject: 'InfluxDB (UDP)',

    cbindData: function() {
	let me = this;
	me.isCreate = !me.serverid;
	me.serverid = me.serverid || "";
	me.url = `/api2/extjs/config/metrics/influxdb-udp/${me.serverid}`;
	me.method = me.isCreate ? 'POST' : 'PUT';
	if (!me.isCreate) {
	    me.subject = `${me.subject}: ${me.serverid}`;
	}
	return {};
    },

    items: [
	{
	    xtype: 'inputpanel',

	    onGetValues: function(values) {
		values.host += `:${values.port}`;
		delete values.port;
		return values;
	    },

	    column1: [
		{
		    xtype: 'pmxDisplayEditField',
		    name: 'name',
		    fieldLabel: gettext('Name'),
		    allowBlank: false,
		    cbind: {
			editable: '{isCreate}',
			value: '{serverid}',
		    },
		},
		{
		    xtype: 'proxmoxtextfield',
		    name: 'host',
		    fieldLabel: gettext('Host'),
		    allowBlank: false,
		},
	    ],

	    column2: [
		{
		    xtype: 'checkbox',
		    name: 'enable',
		    fieldLabel: gettext('Enabled'),
		    inputValue: 1,
		    uncheckedValue: 0,
		    checked: true,
		},
		{
		    xtype: 'proxmoxintegerfield',
		    name: 'port',
		    minValue: 1,
		    maxValue: 65535,
		    fieldLabel: gettext('Port'),
		    allowBlank: false,
		},
	    ],

	    columnB: [
		{
		    xtype: 'proxmoxtextfield',
		    name: 'comment',
		    fieldLabel: gettext('Comment'),
		    cbind: {
			deleteEmpty: '{!isCreate}',
		    },
		},
	    ],

	    advancedColumn1: [
		{
		    xtype: 'proxmoxintegerfield',
		    name: 'mtu',
		    fieldLabel: 'MTU',
		    minValue: 1,
		    emptyText: '1500',
		    submitEmpty: false,
		    cbind: {
			deleteEmpty: '{!isCreate}',
		    },
		},
	    ],
	},
    ],

    initComponent: function() {
	let me = this;
	me.callParent();

	me.load({
	    success: function(response, options) {
		let values = response.result.data;
		let [_match, host, port] = /^(.*):(\d+)$/.exec(values.host) || [];
		values.host = host;
		values.port = port;
		me.setValues(values);
	    },
	});
    },
});
