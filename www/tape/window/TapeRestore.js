Ext.define('PBS.TapeManagement.TapeRestoreWindow', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsTapeRestoreWindow',
    mixins: ['Proxmox.Mixin.CBind'],

    width: 800,
    title: gettext('Restore Media Set'),
    url: '/api2/extjs/tape/restore',
    method: 'POST',
    showTaskViewer: true,
    isCreate: true,

    defaults: {
	labelWidth: 120,
    },

    referenceHolder: true,

    items: [
	{
	    xtype: 'inputpanel',

	    onGetValues: function(values) {
		let me = this;
		let datastores = [];
		if (values.store && values.store !== "") {
		    datastores.push(values.store);
		    delete values.store;
		}

		if (values.mapping) {
		    datastores.push(values.mapping);
		    delete values.mapping;
		}

		values.store = datastores.join(',');

		return values;
	    },

	    column1: [
		{
		    xtype: 'displayfield',
		    fieldLabel: gettext('Media Set'),
		    cbind: {
			value: '{mediaset}',
		    },
		},
		{
		    xtype: 'displayfield',
		    fieldLabel: gettext('Media Set UUID'),
		    name: 'media-set',
		    submitValue: true,
		    cbind: {
			value: '{uuid}',
		    },
		},
		{
		    xtype: 'pbsDriveSelector',
		    fieldLabel: gettext('Drive'),
		    name: 'drive',
		},
	    ],

	    column2: [
		{
		    xtype: 'pbsUserSelector',
		    name: 'notify-user',
		    fieldLabel: gettext('Notify User'),
		    emptyText: gettext('Current User'),
		    value: null,
		    allowBlank: true,
		    skipEmptyText: true,
		    renderer: Ext.String.htmlEncode,
		},
		{
		    xtype: 'pbsUserSelector',
		    name: 'owner',
		    fieldLabel: gettext('Owner'),
		    emptyText: gettext('Current User'),
		    value: null,
		    allowBlank: true,
		    skipEmptyText: true,
		    renderer: Ext.String.htmlEncode,
		},
		{
		    xtype: 'pbsDataStoreSelector',
		    fieldLabel: gettext('Datastore'),
		    reference: 'defaultDatastore',
		    name: 'store',
		    listeners: {
			change: function(field, value) {
			    let me = this;
			    let grid = me.up('window').lookup('mappingGrid');
			    grid.setNeedStores(!value);
			},
		    },
		},
	    ],

	    columnB: [
		{
		    fieldLabel: gettext('Datastore Mapping'),
		    labelWidth: 200,
		    hidden: true,
		    reference: 'mappingLabel',
		    xtype: 'displayfield',
		},
		{
		    xtype: 'pbsDataStoreMappingField',
		    reference: 'mappingGrid',
		    name: 'mapping',
		    defaultBindProperty: 'value',
		    hidden: true,
		},
	    ],
	},
    ],

    setDataStores: function(datastores) {
	let me = this;

	let label = me.lookup('mappingLabel');
	let grid = me.lookup('mappingGrid');
	let defaultField = me.lookup('defaultDatastore');

	if (!datastores || datastores.length <= 1) {
	    label.setVisible(false);
	    grid.setVisible(false);
	    defaultField.setFieldLabel(gettext('Datastore'));
	    defaultField.setAllowBlank(false);
	    defaultField.setEmptyText("");
	    return;
	}

	label.setVisible(true);
	defaultField.setFieldLabel(gettext('Default Datastore'));
	defaultField.setAllowBlank(true);
	defaultField.setEmptyText(Proxmox.Utils.NoneText);

	grid.setDataStores(datastores);
	grid.setVisible(true);
    },

    initComponent: function() {
	let me = this;

	me.callParent();
	if (me.datastores) {
	    me.setDataStores(me.datastores);
	} else {
	    // use timeout so that the window is rendered already
	    // for correct masking
	    setTimeout(function() {
		Proxmox.Utils.API2Request({
		    waitMsgTarget: me,
		    url: `/tape/media/content?media-set=${me.uuid}`,
		    success: function(response, opt) {
			let datastores = {};
			for (const content of response.result.data) {
			    datastores[content.store] = true;
			}
			me.setDataStores(Object.keys(datastores));
		    },
		    failure: function() {
			// ignore failing api call, maybe catalog is missing
			me.setDataStores();
		    },
		});
	    }, 10);
	}
    },
});

Ext.define('PBS.TapeManagement.DataStoreMappingGrid', {
    extend: 'Ext.grid.Panel',
    alias: 'widget.pbsDataStoreMappingField',
    mixins: ['Ext.form.field.Field'],

    getValue: function() {
	let me = this;
	let datastores = [];
	me.getStore().each((rec) => {
	    let source = rec.data.source;
	    let target = rec.data.target;
	    if (target && target !== "") {
		datastores.push(`${source}=${target}`);
	    }
	});

	return datastores.join(',');
    },

    // this determines if we need at least one valid mapping
    needStores: false,

    setNeedStores: function(needStores) {
	let me = this;
	me.needStores = needStores;
	me.checkChange();
	me.validate();
    },

    setValue: function(value) {
	let me = this;
	me.setDataStores(value);
	return me;
    },

    getErrors: function(value) {
	let me = this;
	let error = false;

	if (me.needStores) {
	    error = true;
	    me.getStore().each((rec) => {
		if (rec.data.target) {
		    error = false;
		}
	    });
	}

	if (error) {
	    me.addCls(['x-form-trigger-wrap-default', 'x-form-trigger-wrap-invalid']);
	    let errorMsg = gettext("Need at least one mapping");
	    me.getActionEl().dom.setAttribute('data-errorqtip', errorMsg);

	    return [errorMsg];
	}
	me.removeCls(['x-form-trigger-wrap-default', 'x-form-trigger-wrap-invalid']);
	me.getActionEl().dom.setAttribute('data-errorqtip', "");
	return [];
    },

    setDataStores: function(datastores) {
	let me = this;
	let store = me.getStore();
	let data = [];

	for (const datastore of datastores) {
	    data.push({
		source: datastore,
		target: '',
	    });
	}

	store.setData(data);
    },

    viewConfig: {
	markDirty: false,
    },

    store: { data: [] },

    columns: [
	{
	    text: gettext('Source Datastore'),
	    dataIndex: 'source',
	    flex: 1,
	},
	{
	    text: gettext('Target Datastore'),
	    xtype: 'widgetcolumn',
	    dataIndex: 'target',
	    flex: 1,
	    widget: {
		xtype: 'pbsDataStoreSelector',
		allowBlank: true,
		emptyText: Proxmox.Utils.NoneText,
		listeners: {
		    change: function(selector, value) {
			let me = this;
			let rec = me.getWidgetRecord();
			if (!rec) {
			    return;
			}
			rec.set('target', value);
			me.up('grid').checkChange();
		    },
		},
	    },
	},
    ],
});
