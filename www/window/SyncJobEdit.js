Ext.define('PBS.window.SyncJobEdit', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsSyncJobEdit',
    mixins: ['Proxmox.Mixin.CBind'],

    userid: undefined,

    onlineHelp: 'syncjobs',

    isAdd: true,

    subject: gettext('Sync Job'),

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

    setValues: function(values) {
	let me = this;
	if (values.id && !values.remote) {
	    values.location = 'local';
	} else {
	    values.location = 'remote';
	}
	me.callParent([values]);
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
			PBS.Utils.delete_if_default(values, 'remote');
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
			listeners: {
			    change: function(field, localNs) {
				let me = this;
				let view = me.up('pbsSyncJobEdit');

				let remoteNs = view.down('pbsRemoteNamespaceSelector[name=remote-ns]').getValue();
				let maxDepthField = view.down('field[name=max-depth]');
				maxDepthField.setLimit(localNs, remoteNs);
				maxDepthField.validate();
			    },
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
			xtype: 'radiogroup',
			fieldLabel: gettext('Location'),
			defaultType: 'radiofield',
			items: [
			    {
				boxLabel: 'Local',
				name: 'location',
				inputValue: 'local',
				submitValue: false,
			    },
			    {
				boxLabel: 'Remote',
				name: 'location',
				inputValue: 'remote',
				submitValue: false,
				checked: true,
			    },
			],
			listeners: {
			    change: function(_group, radio) {
				let me = this;
				let form = me.up('pbsSyncJobEdit');
				let nsField = form.down('field[name=remote-ns]');
				let rateLimitField = form.down('field[name=rate-in]');
				let remoteField = form.down('field[name=remote]');
				let storeField = form.down('field[name=remote-store]');

				if (!storeField.value) {
				    nsField.clearValue();
				    nsField.setDisabled(true);
				}

				let isLocalSync = radio.location === 'local';
				rateLimitField.setDisabled(isLocalSync);
				remoteField.allowBlank = isLocalSync;
				remoteField.setDisabled(isLocalSync);
				storeField.setDisabled(!isLocalSync && !remoteField.value);
				if (isLocalSync === !!remoteField.value) {
				    remoteField.clearValue();
				}

				if (isLocalSync) {
				    storeField.setDisabled(false);
				    rateLimitField.setValue(null);
				    storeField.setRemote(null, true);
				} else {
				    storeField.clearValue();
				    remoteField.validate();
				}
			    },
			},
		    },
		    {
			fieldLabel: gettext('Source Remote'),
			xtype: 'pbsRemoteSelector',
			allowBlank: false,
			name: 'remote',
			skipEmptyText: true,
			listeners: {
			    change: function(f, value) {
				let me = this;
				let remoteStoreField = me.up('pbsSyncJobEdit').down('field[name=remote-store]');
				remoteStoreField.setRemote(value);
				let rateLimitField = me.up('pbsSyncJobEdit').down('field[name=rate-in]');
				rateLimitField.setDisabled(!value);
				if (!value) {
				    rateLimitField.setValue(null);
				}
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
			cbind: {
			    datastore: '{datastore}',
			},
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
			    change: function(field, remoteNs) {
				let me = this;
				let view = me.up('pbsSyncJobEdit');

				let remote = view.down('field[name=remote]').getValue();
				let remoteStore = view.down('field[name=remote-store]').getValue();
				me.up('tabpanel').down('pbsGroupFilter').setRemoteNamespace(remote, remoteStore, remoteNs);

				let localNs = view.down('pbsNamespaceSelector[name=ns]').getValue();
				let maxDepthField = view.down('field[name=max-depth]');
				maxDepthField.setLimit(localNs, remoteNs);
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
		    {
			fieldLabel: gettext('Transfer Last'),
			xtype: 'pbsPruneKeepInput',
			name: 'transfer-last',
			emptyText: gettext('all'),
			autoEl: {
			    tag: 'div',
			    'data-qtip': gettext('The maximum amount of snapshots to be transferred (per group)'),
			},
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
