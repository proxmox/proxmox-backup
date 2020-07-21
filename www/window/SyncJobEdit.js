Ext.define('PBS.window.SyncJobEdit', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsSyncJobEdit',
    mixins: ['Proxmox.Mixin.CBind'],

    userid: undefined,

    isAdd: true,

    subject: gettext('SyncJob'),

    fieldDefaults: { labelWidth: 120 },

    cbindData: function(initialConfig) {
	let me = this;

	let baseurl = '/api2/extjs/config/sync';
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
		fieldLabel: gettext('Sync Job ID'),
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
		fieldLabel: gettext('Source Remote'),
		xtype: 'pbsRemoteSelector',
		allowBlank: false,
		name: 'remote',
	    },
	    {
		fieldLabel: gettext('Source Datastore'),
		xtype: 'proxmoxtextfield',
		allowBlank: false,
		name: 'remote-store',
	    },
	    {
		fieldLabel: gettext('Local Datastore'),
		xtype: 'pbsDataStoreSelector',
		allowBlank: false,
		name: 'store',
	    },
	],

	column2: [
	    {
		fieldLabel: gettext('Remove vanished'),
		xtype: 'proxmoxcheckbox',
		name: 'remove-vanished',
		uncheckedValue: false,
		value: true,
	    },
	    {
		fieldLabel: gettext('Schedule'),
		xtype: 'proxmoxtextfield',
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
