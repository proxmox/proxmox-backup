Ext.define('PBS.window.VerifyJobEdit', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsVerifyJobEdit',
    mixins: ['Proxmox.Mixin.CBind'],

    userid: undefined,

    onlineHelp: 'maintenance-verification',

    isAdd: true,

    subject: gettext('Verification Job'),

    fieldDefaults: { labelWidth: 120 },
    defaultFocus: 'field[name="ignore-verified"]',

    cbindData: function(initialConfig) {
	let me = this;

	let baseurl = '/api2/extjs/config/verify';
	let id = initialConfig.id;

	me.isCreate = !id;
	me.url = id ? `${baseurl}/${id}` : baseurl;
	me.method = id ? 'PUT' : 'POST';
	me.scheduleValue = id ? null : 'daily';
	me.autoLoad = !!id;
	me.editDatastore = me.datastore === undefined && me.isCreate;
	return { };
    },

    viewModel: {
	data: {
	    'ignore-verified': true,
	},
    },

    items: {
	xtype: 'inputpanel',
	onGetValues: function(values) {
	    let me = this;

	    if (!values.id && me.up('pbsVerifyJobEdit').isCreate) {
		values.id = 'v-' + Ext.data.identifier.Uuid.Global.generate().slice(0, 13);
	    }
	    return values;
	},
	column1: [
	    {
		xtype: 'pmxDisplayEditField',
		fieldLabel: gettext('Local Datastore'),
		name: 'store',
		submitValue: true,
		cbind: {
		    editable: '{editDatastore}',
		    value: '{datastore}',
		},
		editConfig: {
		    xtype: 'pbsDataStoreSelector',
		    allowBlank: false,
		},
	    },
	    {
		xtype: 'pbsCalendarEvent',
		name: 'schedule',
		fieldLabel: gettext('Schedule'),
		emptyText: gettext('none (disabled)'),
		cbind: {
		    value: '{scheduleValue}',
		    deleteEmpty: '{!isCreate}',
		},
	    },
	],

	column2: [
	    {
		xtype: 'proxmoxcheckbox',
		name: 'ignore-verified',
		fieldLabel: gettext('Skip verified snapshots'),
		labelWidth: 150,
		uncheckedValue: false,
		value: true,
		bind: {
		    value: '{ignore-verified}',
		},
	    },
	    {
		xtype: 'proxmoxintegerfield',
		name: 'outdated-after',
		fieldLabel: gettext('Re-Verify After (days)'),
		labelWidth: 150,
		minValue: 1,
		value: 30,
		allowBlank: true,
		emptyText: gettext('Never'),
		bind: {
		    disabled: '{!ignore-verified}',
		},
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
