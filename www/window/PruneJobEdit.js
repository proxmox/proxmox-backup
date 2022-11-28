Ext.define('PBS.window.PruneJobEdit', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsPruneJobEdit',
    mixins: ['Proxmox.Mixin.CBind'],

    userid: undefined,

    onlineHelp: 'maintenance_prune_jobs',

    isAdd: true,

    subject: gettext('Prune Job'),

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
	    values.disable = !values.enable;
	    delete values.enable;

	    return values;
	},
	onSetValues: function(values) {
	    let me = this;
	    values.enable = !values.disable;
	    delete values.disable;

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
		xtype: 'pbsNamespaceMaxDepthReduced',
		name: 'max-depth',
		fieldLabel: gettext('Max. Depth'),
		deleteEmpty: true,
	    },
	],

	column2: [
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

	columnB: [
	    {
		xtype: 'pbsPruneInputPanel',
		getValues: () => ({}), // let that handle our inputpanel here
	    },
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
