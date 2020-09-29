Ext.define('PBS.window.RemoteEdit', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsRemoteEdit',
    mixins: ['Proxmox.Mixin.CBind'],

    onlineHelp: 'backup_remote',

    userid: undefined,

    isAdd: true,

    subject: gettext('Remote'),

    fieldDefaults: { labelWidth: 120 },

    cbindData: function(initialConfig) {
	let me = this;

	let baseurl = '/api2/extjs/config/remote';
	let name = initialConfig.name;

	me.isCreate = !name;
	me.url = name ? `${baseurl}/${name}` : baseurl;
	me.method = name ? 'PUT' : 'POST';
	me.autoLoad = !!name;
	return {
	    passwordEmptyText: me.isCreate ? '' : gettext('Unchanged'),
	};
    },

    items: {
	xtype: 'inputpanel',
	column1: [
	    {
		xtype: 'pmxDisplayEditField',
		name: 'name',
		fieldLabel: gettext('Remote'),
		renderer: Ext.htmlEncode,
		allowBlank: false,
		minLength: 4,
		cbind: {
		    editable: '{isCreate}',
		},
	    },
	    {
		xtype: 'proxmoxtextfield',
		allowBlank: false,
		name: 'host',
		fieldLabel: gettext('Host'),
	    },
	    {
		xtype: 'proxmoxintegerfield',
		allowBlank: true,
		minValue: 1,
		maxValue: 2**16,
		name: 'port',
		emptyText: 8007,
		deleteEmpty: true,
		fieldLabel: gettext('Port'),
	    },
	],

	column2: [
	    {
		xtype: 'proxmoxtextfield',
		allowBlank: false,
		name: 'userid',
		fieldLabel: gettext('Userid'),
	    },
	    {
		xtype: 'textfield',
		inputType: 'password',
		fieldLabel: gettext('Password'),
		name: 'password',
		cbind: {
		    emptyText: '{passwordEmptyText}',
		    allowBlank: '{!isCreate}',
		},
	    },
	],

	columnB: [
	    {
		xtype: 'proxmoxtextfield',
		name: 'fingerprint',
		deleteEmpty: true,
		fieldLabel: gettext('Fingerprint'),
	    },
	    {
		xtype: 'proxmoxtextfield',
		name: 'comment',
		deleteEmpty: true,
		fieldLabel: gettext('Comment'),
	    },
	],
    },

    getValues: function() {
	let me = this;
	let values = me.callParent(arguments);

	if (values.password === '') {
	    delete values.password;
	}

	return values;
    },
});
