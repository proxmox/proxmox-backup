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
	    'ignoreVerified': true,
	},
    },
    controller: {
	xclass: 'Ext.app.ViewController',
	control: {
	    'pbsDataStoreSelector': {
		change: 'storeChange',
	    },
	},

	storeChange: function(field, value) {
	    let view = this.getView();
	    let nsSelector = view.down('pbsNamespaceSelector');
	    nsSelector.setDatastore(value);
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
		xtype: 'pbsNamespaceSelector',
		name: 'ns',
		fieldLabel: gettext('Namespace'),
		cbind: {
		    datastore: '{datastore}',
		},
	    },
	    {
		xtype: 'pbsNamespaceMaxDepth',
		name: 'max-depth',
		fieldLabel: gettext('Max. Depth'),
		deleteEmpty: true,
	    },
	],

	column2: [
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
	    {
		xtype: 'proxmoxcheckbox',
		name: 'ignore-verified',
		fieldLabel: gettext('Skip Verified'),
		uncheckedValue: false,
		value: true,
		bind: {
		    value: '{ignoreVerified}',
		},
	    },
	    {
		xtype: 'fieldcontainer',
		layout: 'hbox',
		fieldLabel: gettext('Re-Verify After'),
		items: [
		    {
			xtype: 'pbsVerifyOutdatedAfter',
			name: 'outdated-after',
			bind: {
			    disabled: '{!ignoreVerified}',
			},
			cbind: {
			    deleteEmpty: '{!isCreate}',
			},
			flex: 1,
		    },
		    {
			xtype: 'displayfield',
			padding: '0 0 0 5',
			name: 'unit',
			submitValue: false,
			value: gettext('days'),
			bind: {
			    disabled: '{!ignoreVerified}',
			},
		    },
		],
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
	advancedColumn1: [
	    {
		xtype: 'pmxDisplayEditField',
		fieldLabel: gettext('Job ID'),
		emptyText: gettext('Autogenerate'),
		name: 'id',
		allowBlank: true,
		regex: PBS.Utils.SAFE_ID_RE,
		cbind: {
		    editable: '{isCreate}',
		},
	    },
	],
    },
});
