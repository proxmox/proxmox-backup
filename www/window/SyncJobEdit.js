Ext.define('PBS.window.SyncJobEdit', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsSyncJobEdit',
    mixins: ['Proxmox.Mixin.CBind'],

    userid: undefined,

    onlineHelp: 'syncjobs',

    isAdd: true,

    subject: gettext('SyncJob'),

    bodyPadding: 0,

    fieldDefaults: { labelWidth: 120 },
    defaultFocus: 'proxmoxtextfield[name=comment]',

    cbindData: function(initialConfig) {
	let me = this;

	let baseurl = '/api2/extjs/config/sync';
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

		    if (!values.id && me.up('pbsSyncJobEdit').isCreate) {
			values.id = 's-' + Ext.data.identifier.Uuid.Global.generate().slice(0, 13);
		    }
		    if (!me.isCreate) {
			PBS.Utils.delete_if_default(values, 'rate-in');
			if (typeof values.delete === 'string') {
			    values.delete = values.delete.split(',');
			}
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
			fieldLabel: gettext('Local Namespace'),
			name: 'ns',
			cbind: {
			    datastore: '{datastore}',
			},
		    },
		    {
			fieldLabel: gettext('Local Owner'),
			xtype: 'pbsAuthidSelector',
			name: 'owner',
			cbind: {
			    value: '{authid}',
			    deleteEmpty: '{!isCreate}',
			},
		    },
		    {
			fieldLabel: gettext('Sync Schedule'),
			xtype: 'pbsCalendarEvent',
			name: 'schedule',
			emptyText: gettext('none (disabled)'),
			cbind: {
				deleteEmpty: '{!isCreate}',
				value: '{scheduleValue}',
			},
		    },
		    {
			xtype: 'pmxBandwidthField',
			name: 'rate-in',
			fieldLabel: gettext('Rate Limit'),
			emptyText: gettext('Unlimited'),
			submitAutoScaledSizeUnit: true,
			// NOTE: handle deleteEmpty in onGetValues due to bandwidth field having a cbind too
		    },
		],

		column2: [
		    {
			fieldLabel: gettext('Source Remote'),
			xtype: 'pbsRemoteSelector',
			allowBlank: false,
			name: 'remote',
			listeners: {
			    change: function(f, value) {
				let me = this;
				let remoteStoreField = me.up('pbsSyncJobEdit').down('field[name=remote-store]');
				remoteStoreField.setRemote(value);
				let remoteNamespaceField = me.up('pbsSyncJobEdit').down('field[name=remote-ns]');
				remoteNamespaceField.setRemote(value);
			    },
			},
		    },
		    {
			fieldLabel: gettext('Source Datastore'),
			xtype: 'pbsRemoteStoreSelector',
			allowBlank: false,
			autoSelect: false,
			name: 'remote-store',
			disabled: true,
			listeners: {
			    change: function(field, value) {
				let me = this;
				let remoteField = me.up('pbsSyncJobEdit').down('field[name=remote]');
				let remote = remoteField.getValue();
				let remoteNamespaceField = me.up('pbsSyncJobEdit').down('field[name=remote-ns]');
				remoteNamespaceField.setRemote(remote);
				remoteNamespaceField.setRemoteStore(value);
				me.up('tabpanel').down('pbsGroupFilter').setRemoteDatastore(remote, value);
			    },
			},
		    },
		    {
			fieldLabel: gettext('Source Namespace'),
			xtype: 'pbsRemoteNamespaceSelector',
			allowBlank: true,
			autoSelect: false,
			name: 'remote-ns',
			disabled: true,
			listeners: {
			    change: function(field, value) {
				let me = this;
				let remoteField = me.up('pbsSyncJobEdit').down('field[name=remote]');
				let remote = remoteField.getValue();
				let remoteStoreField = me.up('pbsSyncJobEdit').down('field[name=remote-store]');
				let remoteStore = remoteStoreField.getValue();
				me.up('tabpanel').down('pbsGroupFilter').setRemoteNamespace(remote, remoteStore, value);
			    },
			},
		    },
		    {
			xtype: 'pbsNamespaceMaxDepth',
			name: 'max-depth',
			fieldLabel: gettext('Max. Depth'),
			deleteEmpty: true,
		    },
		    {
			fieldLabel: gettext('Remove vanished'),
			xtype: 'proxmoxcheckbox',
			name: 'remove-vanished',
			autoEl: {
			    tag: 'div',
			    'data-qtip': gettext('Remove snapshots from local datastore if they vanished from source datastore?'),
			},
			uncheckedValue: false,
			value: false,
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
	    {
		xtype: 'inputpanel',
		onGetValues: function(values) {
		    PBS.Utils.delete_if_default(values, 'group-filter');
		    if (Ext.isArray(values['group-filter'])) {
			if (values['group-filter'].length === 0) {
			    delete values['group-filter'];
			    values.delete = 'group-filter';
			} else {
			    // merge duplicates
			    values['group-filter'] = [...new Set(values['group-filter'])];
			}
		    }
		    return values;
		},
		title: gettext('Group Filter'),
		items: [
		    {
			xtype: 'pbsGroupFilter',
			name: 'group-filter',
		    },
		],
	    },
	],
    },
});
