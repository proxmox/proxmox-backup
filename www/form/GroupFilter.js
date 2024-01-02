Ext.define('PBS.form.GroupFilter', {
    extend: 'Ext.form.FieldContainer',
    alias: 'widget.pbsGroupFilter',
    mixins: ['Proxmox.Mixin.CBind'],

    cindData: {},

    controller: {
	xclass: 'Ext.app.ViewController',

	removeReferences: function(record) {
	    for (const widget of Object.keys(record.widgets || {})) {
		delete record.widgets[widget].record;
		delete record.widgets[widget];
	    }

	    delete record.widgets;
	},

	cleanupReferences: function(grid) {
	    let me = this;

	    // break cyclic reference
	    grid.getStore()?.each(me.removeReferences);
	},

	removeFilter: function(field) {
	    let me = this;
	    let record = field.getWidgetRecord();
	    if (record === undefined) {
		// this is sometimes called before a record/column is initialized
		return;
	    }

	    // break cyclic reference
	    me.removeReferences(record);

	    me.lookup('grid-include').getStore().remove(record);
	    me.lookup('grid-exclude').getStore().remove(record);
	    me.updateRealField();
	},

	addIncludeFilter: function() {
	    let me = this;
	    me.lookup('grid-include').getStore().add({ behavior: 'include' });
	    me.updateRealField();
	},

	addExcludeFilter: function() {
	    let me = this;
	    me.lookup('grid-exclude').getStore().add({ behavior: 'exclude' });
	    me.updateRealField();
	},


	onBehaviorChange: function(field, value) {
	    let me = this;
	    let record = field.getWidgetRecord();
	    if (record === undefined) {
		return;
	    }

	    record.set('behavior', value);
	    record.commit();
	    if (record.widgets) {
		me.setInputValue(record.widgets, record);
	    }
	    me.updateRealField();
	},

	onTypeChange: function(field, value) {
	    let me = this;
	    let record = field.getWidgetRecord();
	    if (record === undefined) {
		return;
	    }

	    record.set('type', value);
	    record.commit();
	    if (record.widgets) {
		me.setInputValue(record.widgets, record);
	    }
	    me.updateRealField();
	},

	onInputChange: function(field, value) {
	    let me = this;
	    if (value === null) {
		return;
	    }
	    let record = field.record;
	    if (record === undefined) {
		// this is sometimes called before a record/column is initialized
		return;
	    }
	    record.set('input', value);
	    record.commit();

	    me.updateRealField();
	},

	parseGroupFilter: function(filter) {
	    let [, behavior, type, input] = filter.match(/^(?:(exclude|include):)?(type|group|regex):(.*)$/);
	    if (behavior === undefined) {
		behavior = "include";
	    }
	    return {
		behavior,
		type,
		input,
	    };
	},

	onValueChange: function(field, values) {
	    let me = this;
	    let grid_include = me.lookup('grid-include');
	    let grid_exclude = me.lookup('grid-exclude');
	    if (!values || values.length === 0) {
		grid_include.getStore().removeAll();
		grid_exclude.getStore().removeAll();
		return;
	    }
	    let records = values.map((filter) => me.parseGroupFilter(filter));
	    grid_include.getStore().setData(records);
	    grid_exclude.getStore().setData(records);
	},

	setInputValue: function(widgets, rec) {
	    let { type, regex, group } = widgets;

	    type.setHidden(true);
	    type.setDisabled(true);
	    type.setValue(undefined);

	    regex.setHidden(true);
	    regex.setDisabled(true);
	    regex.setValue(undefined);

	    group.setHidden(true);
	    group.setDisabled(true);
	    group.setValue(undefined);

	    let field;
	    if (rec.data.type === 'type') {
		field = type;
	    } else if (rec.data.type === 'regex') {
		field = regex;
	    } else if (rec.data.type === 'group') {
		field = group;
	    } else {
		return;
	    }

	    field.setHidden(false);
	    field.setDisabled(false);
	    field.setValue(rec.data.input);
	},

	newInputColumn: function(col, widget, rec) {
	    let me = this;
	    let view = me.getView();

	    let type = widget.down('pbsGroupTypeSelector');
	    let regex = widget.down('textfield[type=regex]');
	    let group = widget.down('pbsGroupSelector');

	    // cannot reuse the same store for all group selectors due to combo grid limitations,
	    // and just setting the data directly makes trouble due to Ext.util.Collection and its
	    // observers behavior, so, lets do a manual full-clone
	    let recs = [];
	    view.dsStore.each(record => recs.push(record.data));
	    group.getStore().setData(recs);

	    // add a widget reference to the record so we can access them from the other column
	    rec.widgets = {
		type,
		regex,
		group,
	    };

	    // add a record reference so we can access the record from the change handler
	    type.record = rec;
	    regex.record = rec;
	    group.record = rec;

	    // CAUTION: we just created a cyclic reference, we have to delete that on filter removal!

	    me.setInputValue(rec.widgets, rec);
	},

	updateRealField: function() {
	    let me = this;

	    let filter = [];
	    me.lookup('grid-include').getStore().each((rec) => {
	    if (rec.data.type && rec.data.input) {
		filter.push(`${rec.data.type}:${rec.data.input}`);
		}
	    });
	    me.lookup('grid-exclude').getStore().each((rec) => {
		if (rec.data.type && rec.data.input && rec.data.behavior) {
		    let behavior_string = '';
		    if (rec.data.behavior === 'exclude') {
			behavior_string = 'exclude:';
		    }
		    filter.push(`${behavior_string}${rec.data.type}:${rec.data.input}`);
		}
	    });

	    let field = me.lookup('realfield');
	    field.suspendEvent('change');
	    field.setValue(filter);
	    field.resumeEvent('change');
	},

	control: {
	    'grid pbsGroupBehaviorSelector': {
		change: 'onBehaviorChange',
	    },
	    'grid pbsGroupFilterTypeSelector': {
		change: 'onTypeChange',
	    },
	    'grid fieldcontainer field': {
		change: 'onInputChange',
	    },
	    'grid button': {
		click: 'removeFilter',
	    },
	    'field[reference=realfield]': {
		change: 'onValueChange',
	    },
	    'grid': {
		beforedestroy: 'cleanupReferences',
	    },
	},
    },

    onDestroy: function() {
	let me = this;

	me.dsStore.destroy();
	delete me.dsStore;
    },

    setDsStoreUrl: function(url) {
	let me = this;
	me.dsStore.getProxy().setUrl(url);
    },

    updateGroupSelectors: function() {
	let me = this;
	let url;
	if (me.remote) {
	    url = `/api2/json/config/remote/${me.remote}/scan/${me.datastore}/groups`;
	} else if (me.datastore) {
	    url = `/api2/json/admin/datastore/${me.datastore}/groups`;
	} else {
	    return;
	}
	if (me.namespace) {
	    url += `?namespace=${me.namespace}`;
	}
	me.setDsStoreUrl(url);
	me.dsStore.load({
	    callback: (records) => {
		if (me.isDestroyed) {
		    return;
		}
		let groups = records || [];
		me.query('pbsGroupSelector').forEach(el => el.getStore().setData(groups));
	    },
	});
    },

    setLocalDatastore: function(datastore) {
	let me = this;
	if (me.remote === undefined && me.datastore === datastore) {
	    return;
	}
	me.remote = undefined;
	me.datastore = datastore;
	me.updateGroupSelectors();
    },

    setRemoteDatastore: function(remote, datastore) {
	let me = this;
	if (me.remote === remote && me.datastore === datastore && me.namespace === undefined) {
	    return;
	}
	me.remote = remote;
	me.datastore = datastore;
	me.namespace = undefined;
	me.updateGroupSelectors();
    },

    setRemoteNamespace: function(remote, datastore, namespace) {
	let me = this;
	if (me.remote === remote && me.datastore === datastore && me.namespace === namespace) {
	    return;
	}
	me.remote = remote;
	me.datastore = datastore;
	me.namespace = namespace;
	me.updateGroupSelectors();
    },

    items: [
	{
	    xtype: 'pbsGroupFilterGrid',
	    title: 'Include filters',
	    margin: '0 0 5 0',
	    reference: 'grid-include',
	    store: {
		filters: [
		    function(item) {
                    return item.data.behavior === "include";
		    },
		],
            },
	    emptyText: gettext('Include all groups'),
	    viewConfig: {
		deferEmptyText: false,
	    },
	},
	{
	    xtype: 'container',
	    layout: {
		type: 'hbox',
	    },
	    items: [
		{
		    xtype: 'button',
		    text: gettext('Add include'),
		    iconCls: 'fa fa-plus-circle',
		    handler: 'addIncludeFilter',
		},
		{
		    xtype: 'box',
		    flex: 1,
		},
		{
		    xtype: 'box',
		    style: 'margin: 3px 0px;',
		    html: `<span class="pmx-hint">${gettext('Note')}</span>: `
			+ gettext('Filters are additive'),
		},
	    ],
	},
	{
	    xtype: 'pbsGroupFilterGrid',
	    title: 'Exclude filters',
	    margin: '10 0 5 0',
	    reference: 'grid-exclude',
	    store: {
		filters: [
		    function(item) {
                    return item.data.behavior === "exclude";
		    },
		],
            },
	},
	{
	    xtype: 'hiddenfield',
	    reference: 'realfield',
	    setValue: function(value) {
		let me = this;
		me.value = value;
		me.checkChange();
	    },
	    getValue: function() {
		return this.value;
	    },
	    getSubmitValue: function() {
		return this.value;
	    },
	    cbind: {
		name: '{name}',
	    },
	},
	{
	    xtype: 'container',
	    layout: {
		type: 'hbox',
	    },
	    items: [
		{
		    xtype: 'button',
		    text: gettext('Add exclude'),
		    iconCls: 'fa fa-plus-circle',
		    handler: 'addExcludeFilter',
		},
		{
		    xtype: 'box',
		    flex: 1,
		},
		{
		    xtype: 'box',
		    style: 'margin: 3px 0px;',
		    html: `<span class="pmx-hint">${gettext('Note')}</span>: `
			+ gettext('Exclude filters will be applied after include filters'),
		},
	    ],
	},
    ],

    initComponent: function() {
	let me = this;
	me.callParent();
	me.dsStore = Ext.create('Ext.data.Store', {
	    sorters: 'group',
	    model: 'pbs-groups',
	});
    },
});

Ext.define('PBS.form.pbsGroupBehaviorSelector', {
    extend: 'Proxmox.form.KVComboBox',
    alias: 'widget.pbsGroupBehaviorSelector',

    allowBlank: false,

    comboItems: [
	['include', gettext('Include')],
	['exclude', gettext('Exclude')],
    ],
});
Ext.define('PBS.form.GroupFilterGrid', {
    extend: 'Ext.grid.Panel',
    alias: 'widget.pbsGroupFilterGrid',

    scrollable: true,
    height: 200,
    store: {
	fields: ['type', 'input'],
            },
    columns: [
	{
	    text: gettext('Filter Type'),
	    xtype: 'widgetcolumn',
	    dataIndex: 'type',
	    flex: 1,
	    widget: {
		xtype: 'pbsGroupFilterTypeSelector',
		isFormField: false,
	    },
	},
	{
	    text: gettext('Filter Value'),
	    xtype: 'widgetcolumn',
	    flex: 1,
	    onWidgetAttach: 'newInputColumn',
	    widget: {
		padding: 0,
		bodyPadding: 0,
		xtype: 'fieldcontainer',
		layout: 'fit',
		defaults:
		{
		    margin: 0,
		},
		items: [
		    {
			hidden: true,
			xtype: 'pbsGroupTypeSelector',
			isFormField: false,
		    },
		    {
			hidden: true,
			xtype: 'textfield',
			type: 'regex',
			isFormField: false,
		    },
		    {
			hidden: true,
			xtype: 'pbsGroupSelector',
			isFormField: false,
		    },
		],
	    },
	},
	{
	    xtype: 'widgetcolumn',
	    width: 40,
	    widget: {
		xtype: 'button',
		iconCls: 'fa fa-trash-o',
	    },
	},
    ],
});

Ext.define('PBS.form.GroupFilterTypeSelector', {
    extend: 'Proxmox.form.KVComboBox',
    alias: 'widget.pbsGroupFilterTypeSelector',

    allowBlank: false,

    comboItems: [
	['type', gettext('Type')],
	['group', gettext('Group')],
	['regex', gettext('Regex')],
    ],
});

Ext.define('PBS.form.GroupTypeSelector', {
    extend: 'Proxmox.form.KVComboBox',
    alias: 'widget.pbsGroupTypeSelector',

    allowBlank: false,

    comboItems: [
	['vm', gettext('VM')],
	['ct', gettext('CT')],
	['host', gettext('Host')],
    ],
});
