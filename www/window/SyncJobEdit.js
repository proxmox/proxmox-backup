Ext.define('PBS.form.RemoteStoreSelector', {
    extend: 'Proxmox.form.ComboGrid',
    alias: 'widget.pbsRemoteStoreSelector',

    queryMode: 'local',

    valueField: 'store',
    displayField: 'store',
    notFoundIsValid: true,

    matchFieldWidth: false,
    listConfig: {
	loadingText: gettext('Scanning...'),
	width: 350,
	columns: [
	    {
		header: gettext('Datastore'),
		sortable: true,
		dataIndex: 'store',
		renderer: Ext.String.htmlEncode,
		flex: 1,
	    },
	    {
		header: gettext('Comment'),
		dataIndex: 'comment',
		renderer: Ext.String.htmlEncode,
		flex: 1,
	    },
	],
    },

    doRawQuery: function() {
	// do nothing.
    },

    setRemote: function(remote) {
	let me = this;

	if (me.remote === remote) {
	    return;
	}

	me.remote = remote;

	let store = me.store;
	store.removeAll();

	if (me.remote) {
	    me.setDisabled(false);
	    if (!me.firstLoad) {
		me.clearValue();
	    }

	    store.proxy.url = '/api2/json/config/remote/' + encodeURIComponent(me.remote) + '/scan';
	    store.load();

	    me.firstLoad = false;
	} else {
	    me.setDisabled(true);
	    me.clearValue();
	}
    },

    initComponent: function() {
	let me = this;

	me.firstLoad = true;

	let store = Ext.create('Ext.data.Store', {
	    fields: ['store', 'comment'],
	    proxy: {
		type: 'proxmox',
		url: '/api2/json/config/remote/' + encodeURIComponent(me.remote) + '/scan',
	    },
	});

	store.sort('store', 'ASC');

	Ext.apply(me, {
	    store: store,
	});

	me.callParent();
    },
});


Ext.define('PBS.window.SyncJobEdit', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsSyncJobEdit',
    mixins: ['Proxmox.Mixin.CBind'],

    userid: undefined,

    onlineHelp: 'syncjobs',

    isAdd: true,

    subject: gettext('SyncJob'),

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

    items: {
	xtype: 'inputpanel',
	onGetValues: function(values) {
	    let me = this;

	    if (!values.id && me.up('pbsSyncJobEdit').isCreate) {
		values.id = 's-' + Ext.data.identifier.Uuid.Global.generate().slice(0, 13);
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
		fieldLabel: gettext('Local Owner'),
		xtype: 'pbsAuthidSelector',
		name: 'owner',
		cbind: {
		    value: '{authid}',
		    deleteEmpty: '{!isCreate}',
		},
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
	],

	columnB: [
	    {
		fieldLabel: gettext('Backup Groups'),
		xtype: 'displayfield',
		name: 'groups',
		renderer: v => v ? Ext.String.htmlEncode(v) : gettext('All'),
		cbind: {
		    hidden: '{isCreate}',
		},
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
    },
});
