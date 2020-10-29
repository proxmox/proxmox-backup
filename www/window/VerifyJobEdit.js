Ext.define('PBS.window.VerifyJobEdit', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsVerifyJobEdit',
    mixins: ['Proxmox.Mixin.CBind'],

    userid: undefined,

    onlineHelp: 'verifyjobs',

    isAdd: true,

    subject: gettext('VerifyJob'),

    fieldDefaults: { labelWidth: 120 },
    defaultFocus: 'field[name="ignore-verified"]',

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
		values.id = 'auto-' + Ext.data.identifier.Uuid.Global.generate().slice(0, 23);
	    }
	    return values;
	},
	column1: [
	    {
		xtype: 'displayfield',
		name: 'store',
		fieldLabel: gettext('Datastore'),
		allowBlank: false,
		submitValue: true,
		cbind: {
		    value: '{datastore}',
		},
	    },
	    {
		xtype: 'pbsCalendarEvent',
		name: 'schedule',
		fieldLabel: gettext('Schedule'),
		emptyText: gettext('none (disabled)'),
		value: 'daily',
		cbind: {
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
