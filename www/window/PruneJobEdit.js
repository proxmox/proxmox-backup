Ext.define('PBS.window.PruneJobEdit', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsPruneJobEdit',
    mixins: ['Proxmox.Mixin.CBind'],

    userid: undefined,

    onlineHelp: 'prunejobs',

    isAdd: true,

    subject: gettext('Prune Job'),

    bodyPadding: 0,

    fieldDefaults: { labelWidth: 120 },
    defaultFocus: 'proxmoxtextfield[name=comment]',

    cbindData: function(initialConfig) {
	let me = this;

	let baseurl = '/api2/extjs/config/prune';
	let id = initialConfig.id;

	me.isCreate = !id;
	me.url = id ? `${baseurl}/${id}` : baseurl;
	me.method = id ? 'PUT' : 'POST';
	me.autoLoad = !!id;
	me.scheduleValue = id ? null : 'hourly';
	me.authid = id ? null : Proxmox.UserName;
	me.editDatastore = me.datastore === undefined && me.isCreate;
	return { };
    },

    controller: {
	xclass: 'Ext.app.ViewController',
	control: {
	    'pbsDataStoreSelector[name=store]': {
		change: 'storeChange',
	    },
	},

	storeChange: function(field, value) {
	    let view = this.getView();
	    let nsSelector = view.down('pbsNamespaceSelector[name=ns]');
	    nsSelector.setDatastore(value);
	},
    },


    items: {
	xtype: 'tabpanel',
	bodyPadding: 10,
	border: 0,
	items: [
	    {
		title: 'Options',
		xtype: 'inputpanel',
		onGetValues: function(values) {
		    let me = this;

		    if (!values.id && me.up('pbsPruneJobEdit').isCreate) {
			values.id = 's-' + Ext.data.identifier.Uuid.Global.generate().slice(0, 13);
		    }
		    if (!me.isCreate) {
			if (typeof values.delete === 'string') {
			    values.delete = values.delete.split(',');
			}
		    }
		    return values;
		},
		column1: [
		    {
			xtype: 'pmxDisplayEditField',
			fieldLabel: gettext('Datastore'),
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
			fieldLabel: gettext('Namespace'),
			name: 'ns',
			cbind: {
			    datastore: '{datastore}',
			},
			listeners: {
			    change: function(field, localNs) {
				let me = this;
				let view = me.up('pbsPruneJobEdit');

				let maxDepthField = view.down('field[name=max-depth]');
				maxDepthField.setLimit(localNs);
				maxDepthField.validate();
			    },
			},
		    },
		    {
			fieldLabel: gettext('Prune Schedule'),
			xtype: 'pbsCalendarEvent',
			name: 'schedule',
			emptyText: gettext('none (disabled)'),
			cbind: {
				deleteEmpty: '{!isCreate}',
				value: '{scheduleValue}',
			},
		    },
		    {
			xtype: 'proxmoxcheckbox',
			fieldLabel: gettext('Enabled'),
			name: 'enable',
			uncheckedValue: 0,
			defaultValue: 1,
			checked: true,
		    },
		],

		column2: [
		    {
			xtype: 'pbsNamespaceMaxDepthReduced',
			name: 'max-depth',
			fieldLabel: gettext('Max. Depth'),
			deleteEmpty: true,
		    },
		    {
			xtype: 'pbsPruneKeepInput',
			name: 'keep-last',
			fieldLabel: gettext('Keep Last'),
			deleteEmpty: true,
		    },
		    {
			xtype: 'pbsPruneKeepInput',
			name: 'keep-hourly',
			fieldLabel: gettext('Keep Hourly'),
			deleteEmpty: true,
		    },
		    {
			xtype: 'pbsPruneKeepInput',
			name: 'keep-daily',
			fieldLabel: gettext('Keep Daily'),
			deleteEmpty: true,
		    },
		    {
			xtype: 'pbsPruneKeepInput',
			name: 'keep-weekly',
			fieldLabel: gettext('Keep Weekly'),
			deleteEmpty: true,
		    },
		    {
			xtype: 'pbsPruneKeepInput',
			name: 'keep-monthly',
			fieldLabel: gettext('Keep Monthly'),
			deleteEmpty: true,
		    },
		    {
			xtype: 'pbsPruneKeepInput',
			name: 'keep-yearly',
			fieldLabel: gettext('Keep Yearly'),
			deleteEmpty: true,
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
	],
    },
});
