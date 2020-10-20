Ext.define('PBS.window.VerifyJobEdit', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsVerifyJobEdit',
    mixins: ['Proxmox.Mixin.CBind'],

    userid: undefined,

    onlineHelp: 'verifyjobs',

    isAdd: true,

    subject: gettext('VerifyJob'),

    fieldDefaults: { labelWidth: 120 },

    cbindData: function(initialConfig) {
	let me = this;

	let baseurl = '/api2/extjs/config/verify';
	let id = initialConfig.id;

	me.isCreate = !id;
	me.url = id ? `${baseurl}/${id}` : baseurl;
	me.method = id ? 'PUT' : 'POST';
	me.autoLoad = !!id;
	return { };
    },

    items: {
	xtype: 'inputpanel',
	column1: [
	    {
		fieldLabel: gettext('Verify Job ID'),
		xtype: 'pmxDisplayEditField',
		name: 'id',
		renderer: Ext.htmlEncode,
		allowBlank: false,
		minLength: 4,
		cbind: {
		    editable: '{isCreate}',
		},
	    },
	    {
		fieldLabel: gettext('Datastore'),
		xtype: 'pbsDataStoreSelector',
		allowBlank: false,
		name: 'store',
	    },
	    {
		xtype: 'proxmoxintegerfield',
		fieldLabel: gettext('Days valid'),
		minValue: 1,
		value: '',
		allowBlank: true,
		name: 'outdated-after',
		emptyText: gettext('no expiration'),
		cbind: {
		    deleteEmpty: '{!isCreate}',
		},
	    },
	],

	column2: [
	    {
		fieldLabel: gettext('Ignore verified'),
		xtype: 'proxmoxcheckbox',
		name: 'ignore-verified',
		uncheckedValue: false,
		value: true,
	    },
	    {
		fieldLabel: gettext('Schedule'),
		xtype: 'pbsCalendarEvent',
		name: 'schedule',
		emptyText: gettext('none'),
		cbind: {
		    deleteEmpty: '{!isCreate}',
		},
	    },
	],

	columnB: [
	    {
		fieldLabel: gettext('Comment'),
		xtype: 'proxmoxtextfield',
		name: 'comment',
		cbind: {
		    deleteEmpty: '{!isCreate}',
		},
	    },
	],
    },
});
