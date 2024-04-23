// Override some components from widget toolkit.
// This was done so that we can already use the improved UI for editing
// match rules without waiting for the needed API calls in PVE to be merged
//
// This can and *should* be removed once these changes have landed in
// widget toolkit:
// https://lists.proxmox.com/pipermail/pve-devel/2024-April/063539.html


Ext.define('pbs-notification-fields', {
    extend: 'Ext.data.Model',
    fields: ['name', 'description'],
    idProperty: 'name',
});

Ext.define('pbs-notification-field-values', {
    extend: 'Ext.data.Model',
    fields: ['value', 'comment', 'field'],
    idProperty: 'value',
});

Ext.define('PBS.panel.NotificationRulesEditPanel', {
    override: 'Proxmox.panel.NotificationRulesEditPanel',
    extend: 'Proxmox.panel.InputPanel',
    xtype: 'pmxNotificationMatchRulesEditPanel',
    mixins: ['Proxmox.Mixin.CBind'],

    controller: {
	xclass: 'Ext.app.ViewController',

	// we want to also set the empty value, but 'bind' does not do that so
	// we have to set it then (and only then) to get the correct value in
	// the tree
	control: {
	    'field': {
		change: function(cmp) {
		    let me = this;
		    let vm = me.getViewModel();
		    if (cmp.field) {
			let record = vm.get('selectedRecord');
			if (!record) {
			    return;
			}
			let data = Ext.apply({}, record.get('data'));
			let value = cmp.getValue();
			// only update if the value is empty (or empty array)
			if (!value || !value.length) {
			    data[cmp.field] = value;
			    record.set({ data });
			}
		    }
		},
	    },
	},
    },

    viewModel: {
	data: {
	    selectedRecord: null,
	    matchFieldType: 'exact',
	    matchFieldField: '',
	    matchFieldValue: '',
	    rootMode: 'all',
	},

	formulas: {
	    nodeType: {
		get: function(get) {
		    let record = get('selectedRecord');
		    return record?.get('type');
		},
		set: function(value) {
		    let me = this;
		    let record = me.get('selectedRecord');

		    let data;

		    switch (value) {
			case 'match-severity':
			    data = {
				value: ['info', 'notice', 'warning', 'error', 'unknown'],
			    };
			    break;
			case 'match-field':
			    data = {
				type: 'exact',
				field: '',
				value: '',
			    };
			    break;
			case 'match-calendar':
			    data = {
				value: '',
			    };
			    break;
		    }

		    let node = {
			type: value,
			data,
		    };
		    record.set(node);
		},
	    },
	    showMatchingMode: function(get) {
		let record = get('selectedRecord');
		if (!record) {
		    return false;
		}
		return record.isRoot();
	    },
	    showMatcherType: function(get) {
		let record = get('selectedRecord');
		if (!record) {
		    return false;
		}
		return !record.isRoot();
	    },

	    rootMode: {
		bind: {
		    bindTo: '{selectedRecord}',
		    deep: true,
		},
		set: function(value) {
		    let me = this;
		    let record = me.get('selectedRecord');
		    let currentData = record.get('data');
		    let invert = false;
		    if (value.startsWith('not')) {
			value = value.substring(3);
			invert = true;
		    }
		    record.set({
			data: {
			    ...currentData,
			    value,
			    invert,
			},
		    });
		},
		get: function(record) {
		    let prefix = record?.get('data').invert ? 'not' : '';
		    return prefix + record?.get('data')?.value;
		},
	    },
	},
    },

    column1: [
	{
	    xtype: 'pbsNotificationMatchRuleTree',
	    cbind: {
		isCreate: '{isCreate}',
	    },
	},
    ],
    column2: [
	{
	    xtype: 'pbsNotificationMatchRuleSettings',
	    cbind: {
		baseUrl: '{baseUrl}',
	    },
	},

    ],

    onGetValues: function(values) {
	let me = this;

	let deleteArrayIfEmtpy = (field) => {
	    if (Ext.isArray(values[field])) {
		if (values[field].length === 0) {
		    delete values[field];
		    if (!me.isCreate) {
			Proxmox.Utils.assemble_field_data(values, { 'delete': field });
		    }
		}
	    }
	};
	deleteArrayIfEmtpy('match-field');
	deleteArrayIfEmtpy('match-severity');
	deleteArrayIfEmtpy('match-calendar');

	return values;
    },
});

Ext.define('PBS.panel.NotificationMatchRuleTree', {
    extend: 'Ext.panel.Panel',
    xtype: 'pbsNotificationMatchRuleTree',
    mixins: ['Proxmox.Mixin.CBind'],
    border: false,

    getNodeTextAndIcon: function(type, data) {
	let text;
	let iconCls;

	switch (type) {
	    case 'match-severity': {
		let v = data.value;
		if (Ext.isArray(data.value)) {
		    v = data.value.join(', ');
		}
		text = Ext.String.format(gettext("Match severity: {0}"), v);
		iconCls = 'fa fa-exclamation';
		if (!v) {
		    iconCls += ' internal-error';
		}
	    } break;
	    case 'match-field': {
		let field = data.field;
		let value = data.value;
		text = Ext.String.format(gettext("Match field: {0}={1}"), field, value);
		iconCls = 'fa fa-square-o';
		if (!field || !value || (Ext.isArray(value) && !value.length)) {
		    iconCls += ' internal-error';
		}
	    } break;
	    case 'match-calendar': {
		let v = data.value;
		text = Ext.String.format(gettext("Match calendar: {0}"), v);
		iconCls = 'fa fa-calendar-o';
		if (!v || !v.length) {
		    iconCls += ' internal-error';
		}
	    } break;
	    case 'mode':
		if (data.value === 'all') {
		    text = gettext("All");
		} else if (data.value === 'any') {
		    text = gettext("Any");
		}
		if (data.invert) {
		    text = `!${text}`;
		}
		iconCls = 'fa fa-filter';

		break;
	}

	return [text, iconCls];
    },

    initComponent: function() {
	let me = this;

	let treeStore = Ext.create('Ext.data.TreeStore', {
	    root: {
		expanded: true,
		expandable: false,
		text: '',
		type: 'mode',
		data: {
		    value: 'all',
		    invert: false,
		},
		children: [],
		iconCls: 'fa fa-filter',
	    },
	});

	let realMatchFields = Ext.create({
	    xtype: 'hiddenfield',
	    setValue: function(value) {
		this.value = value;
		this.checkChange();
	    },
	    getValue: function() {
		return this.value;
	    },
	    getErrors: function() {
		for (const matcher of this.value ?? []) {
		    let matches = matcher.match(/^([^:]+):([^=]+)=(.+)$/);
		    if (!matches) {
			return [""]; // fake error for validation
		    }
		}
		return [];
	    },
	    getSubmitValue: function() {
		let value = this.value;
		if (!value) {
		    value = [];
		}
		return value;
	    },
	    name: 'match-field',
	});

	let realMatchSeverity = Ext.create({
	    xtype: 'hiddenfield',
	    setValue: function(value) {
		this.value = value;
		this.checkChange();
	    },
	    getValue: function() {
		return this.value;
	    },
	    getErrors: function() {
		for (const severities of this.value ?? []) {
		    if (!severities) {
			return [""]; // fake error for validation
		    }
		}
		return [];
	    },
	    getSubmitValue: function() {
		let value = this.value;
		if (!value) {
		    value = [];
		}
		return value;
	    },
	    name: 'match-severity',
	});

	let realMode = Ext.create({
	    xtype: 'hiddenfield',
	    name: 'mode',
	    setValue: function(value) {
		this.value = value;
		this.checkChange();
	    },
	    getValue: function() {
		return this.value;
	    },
	    getSubmitValue: function() {
		let value = this.value;
		return value;
	    },
	});

	let realMatchCalendar = Ext.create({
	    xtype: 'hiddenfield',
	    name: 'match-calendar',

	    setValue: function(value) {
		this.value = value;
		this.checkChange();
	    },
	    getValue: function() {
		return this.value;
	    },
	    getErrors: function() {
		for (const timespan of this.value ?? []) {
		    if (!timespan) {
			return [""]; // fake error for validation
		    }
		}
		return [];
	    },
	    getSubmitValue: function() {
		let value = this.value;
		return value;
	    },
	});

	let realInvertMatch = Ext.create({
	    xtype: 'proxmoxcheckbox',
	    name: 'invert-match',
	    hidden: true,
	    deleteEmpty: !me.isCreate,
	});

	let storeChanged = function(store) {
	    store.suspendEvent('datachanged');

	    let matchFieldStmts = [];
	    let matchSeverityStmts = [];
	    let matchCalendarStmts = [];
	    let modeStmt = 'all';
	    let invertMatchStmt = false;

	    store.each(function(model) {
		let type = model.get('type');
		let data = model.get('data');

		switch (type) {
		    case 'match-field':
			matchFieldStmts.push(`${data.type}:${data.field ?? ''}=${data.value ?? ''}`);
			break;
		    case 'match-severity':
			if (Ext.isArray(data.value)) {
			    matchSeverityStmts.push(data.value.join(','));
			} else {
			    matchSeverityStmts.push(data.value);
			}
			break;
		    case 'match-calendar':
			matchCalendarStmts.push(data.value);
			break;
		    case 'mode':
			modeStmt = data.value;
			invertMatchStmt = data.invert;
			break;
		}

		let [text, iconCls] = me.getNodeTextAndIcon(type, data);
		model.set({
		    text,
		    iconCls,
		});
	    });

	    realMatchFields.suspendEvent('change');
	    realMatchFields.setValue(matchFieldStmts);
	    realMatchFields.resumeEvent('change');

	    realMatchCalendar.suspendEvent('change');
	    realMatchCalendar.setValue(matchCalendarStmts);
	    realMatchCalendar.resumeEvent('change');

	    realMode.suspendEvent('change');
	    realMode.setValue(modeStmt);
	    realMode.resumeEvent('change');

	    realInvertMatch.suspendEvent('change');
	    realInvertMatch.setValue(invertMatchStmt);
	    realInvertMatch.resumeEvent('change');

	    realMatchSeverity.suspendEvent('change');
	    realMatchSeverity.setValue(matchSeverityStmts);
	    realMatchSeverity.resumeEvent('change');

	    store.resumeEvent('datachanged');
	};

	realMatchFields.addListener('change', function(field, value) {
	    let parseMatchField = function(filter) {
		let [, type, matchedField, matchedValue] =
		    filter.match(/^(?:(regex|exact):)?([A-Za-z0-9_][A-Za-z0-9._-]*)=(.+)$/);
		if (type === undefined) {
		    type = "exact";
		}

		if (type === 'exact') {
		    matchedValue = matchedValue.split(',');
		}

		return {
		    type: 'match-field',
		    data: {
			type,
			field: matchedField,
			value: matchedValue,
		    },
		    leaf: true,
		};
	    };

	    for (let node of treeStore.queryBy(
		record => record.get('type') === 'match-field',
	    ).getRange()) {
		node.remove(true);
	    }

	    if (!value) {
		return;
	    }
	    let records = value.map(parseMatchField);

	    let rootNode = treeStore.getRootNode();

	    for (let record of records) {
		rootNode.appendChild(record);
	    }
	});

	realMatchSeverity.addListener('change', function(field, value) {
	    let parseSeverity = function(severities) {
		return {
		    type: 'match-severity',
		    data: {
			value: severities.split(','),
		    },
		    leaf: true,
		};
	    };

	    for (let node of treeStore.queryBy(
		record => record.get('type') === 'match-severity').getRange()) {
		node.remove(true);
	    }

	    let records = value.map(parseSeverity);
	    let rootNode = treeStore.getRootNode();

	    for (let record of records) {
		rootNode.appendChild(record);
	    }
	});

	realMatchCalendar.addListener('change', function(field, value) {
	    let parseCalendar = function(timespan) {
		return {
		    type: 'match-calendar',
		    data: {
			value: timespan,
		    },
		    leaf: true,
		};
	    };

	    for (let node of treeStore.queryBy(
		record => record.get('type') === 'match-calendar').getRange()) {
		node.remove(true);
	    }

	    let records = value.map(parseCalendar);
	    let rootNode = treeStore.getRootNode();

	    for (let record of records) {
		rootNode.appendChild(record);
	    }
	});

	realMode.addListener('change', function(field, value) {
	    let data = treeStore.getRootNode().get('data');
	    treeStore.getRootNode().set('data', {
		...data,
		value,
	    });
	});

	realInvertMatch.addListener('change', function(field, value) {
	    let data = treeStore.getRootNode().get('data');
	    treeStore.getRootNode().set('data', {
		...data,
		invert: value,
	    });
	});

	treeStore.addListener('datachanged', storeChanged);

	let treePanel = Ext.create({
	    xtype: 'treepanel',
	    store: treeStore,
	    minHeight: 300,
	    maxHeight: 300,
	    scrollable: true,

	    bind: {
		selection: '{selectedRecord}',
	    },
	});

	let addNode = function() {
	    let node = {
		type: 'match-field',
		data: {
		    type: 'exact',
		    field: '',
		    value: '',
		},
		leaf: true,
	    };
	    treeStore.getRootNode().appendChild(node);
	    treePanel.setSelection(treeStore.getRootNode().lastChild);
	};

	let deleteNode = function() {
	    let selection = treePanel.getSelection();
	    for (let selected of selection) {
		if (!selected.isRoot()) {
		    selected.remove(true);
		}
	    }
	};

	Ext.apply(me, {
	    items: [
		realMatchFields,
		realMode,
		realMatchSeverity,
		realInvertMatch,
		realMatchCalendar,
		treePanel,
		{
		    xtype: 'button',
		    margin: '5 5 5 0',
		    text: gettext('Add'),
		    iconCls: 'fa fa-plus-circle',
		    handler: addNode,
		},
		{
		    xtype: 'button',
		    margin: '5 5 5 0',
		    text: gettext('Remove'),
		    iconCls: 'fa fa-minus-circle',
		    handler: deleteNode,
		},
	    ],
	});
	me.callParent();
    },
});

Ext.define('PBS.panel.NotificationMatchRuleSettings', {
    extend: 'Ext.panel.Panel',
    xtype: 'pbsNotificationMatchRuleSettings',
    mixins: ['Proxmox.Mixin.CBind'],
    border: false,
    layout: 'anchor',

    items: [
	{
	    xtype: 'proxmoxKVComboBox',
	    name: 'mode',
	    fieldLabel: gettext('Match if'),
	    allowBlank: false,
	    isFormField: false,

	    matchFieldWidth: false,

	    comboItems: [
		['all', gettext('All rules match')],
		['any', gettext('Any rule matches')],
		['notall', gettext('At least one rule does not match')],
		['notany', gettext('No rule matches')],
	    ],
	    // Hide initially to avoid glitches when opening the window
	    hidden: true,
	    bind: {
		hidden: '{!showMatchingMode}',
		disabled: '{!showMatchingMode}',
		value: '{rootMode}',
	    },
	},
	{
	    xtype: 'proxmoxKVComboBox',
	    fieldLabel: gettext('Node type'),
	    isFormField: false,
	    allowBlank: false,
	    // Hide initially to avoid glitches when opening the window
	    hidden: true,
	    bind: {
		value: '{nodeType}',
		hidden: '{!showMatcherType}',
		disabled: '{!showMatcherType}',
	    },

	    comboItems: [
		['match-field', gettext('Match Field')],
		['match-severity', gettext('Match Severity')],
		['match-calendar', gettext('Match Calendar')],
	    ],
	},
	{
	    xtype: 'pbsNotificationMatchFieldSettings',
	    cbind: {
		baseUrl: '{baseUrl}',
	    },
	},
	{
	    xtype: 'pbsNotificationMatchSeveritySettings',
	},
	{
	    xtype: 'pbsNotificationMatchCalendarSettings',
	},
    ],
});

Ext.define('PBS.panel.MatchCalendarSettings', {
    extend: 'Ext.panel.Panel',
    xtype: 'pbsNotificationMatchCalendarSettings',
    border: false,
    layout: 'anchor',
    // Hide initially to avoid glitches when opening the window
    hidden: true,
    bind: {
	hidden: '{!typeIsMatchCalendar}',
    },
    viewModel: {
	// parent is set in `initComponents`
	formulas: {
	    typeIsMatchCalendar: {
		bind: {
		    bindTo: '{selectedRecord}',
		    deep: true,
		},
		get: function(record) {
		    return record?.get('type') === 'match-calendar';
		},
	    },

	    matchCalendarValue: {
		bind: {
		    bindTo: '{selectedRecord}',
		    deep: true,
		},
		set: function(value) {
		    let me = this;
		    let record = me.get('selectedRecord');
		    let currentData = record.get('data');
		    record.set({
			data: {
			    ...currentData,
			    value: value,
			},
		    });
		},
		get: function(record) {
		    return record?.get('data')?.value;
		},
	    },
	},
    },
    items: [
	{
	    xtype: 'proxmoxKVComboBox',
	    fieldLabel: gettext('Timespan to match'),
	    isFormField: false,
	    allowBlank: false,
	    editable: true,
	    displayField: 'key',
	    field: 'value',
	    bind: {
		value: '{matchCalendarValue}',
		disabled: '{!typeIsMatchCalender}',
	    },

	    comboItems: [
		['mon 8-12', ''],
		['tue..fri,sun 0:00-23:59', ''],
	    ],
	},
    ],

    initComponent: function() {
	let me = this;
	Ext.apply(me.viewModel, {
	    parent: me.up('pmxNotificationMatchRulesEditPanel').getViewModel(),
	});
	me.callParent();
    },
});

Ext.define('PBS.panel.MatchSeveritySettings', {
    extend: 'Ext.panel.Panel',
    xtype: 'pbsNotificationMatchSeveritySettings',
    border: false,
    layout: 'anchor',
    // Hide initially to avoid glitches when opening the window
    hidden: true,
    bind: {
	hidden: '{!typeIsMatchSeverity}',
    },
    viewModel: {
	// parent is set in `initComponents`
	formulas: {
	    typeIsMatchSeverity: {
		bind: {
		    bindTo: '{selectedRecord}',
		    deep: true,
		},
		get: function(record) {
		    return record?.get('type') === 'match-severity';
		},
	    },
	    matchSeverityValue: {
		bind: {
		    bindTo: '{selectedRecord}',
		    deep: true,
		},
		set: function(value) {
		    let record = this.get('selectedRecord');
		    let currentData = record.get('data');
		    record.set({
			data: {
			    ...currentData,
			    value: value,
			},
		    });
		},
		get: function(record) {
		    return record?.get('data')?.value;
		},
	    },
	},
    },
    items: [
	{
	    xtype: 'proxmoxKVComboBox',
	    fieldLabel: gettext('Severities to match'),
	    isFormField: false,
	    allowBlank: true,
	    multiSelect: true,
	    field: 'value',
	    // Hide initially to avoid glitches when opening the window
	    hidden: true,
	    bind: {
		value: '{matchSeverityValue}',
		hidden: '{!typeIsMatchSeverity}',
		disabled: '{!typeIsMatchSeverity}',
	    },

	    comboItems: [
		['info', gettext('Info')],
		['notice', gettext('Notice')],
		['warning', gettext('Warning')],
		['error', gettext('Error')],
		['unknown', gettext('Unknown')],
	    ],
	},
    ],

    initComponent: function() {
	let me = this;
	Ext.apply(me.viewModel, {
	    parent: me.up('pmxNotificationMatchRulesEditPanel').getViewModel(),
	});
	me.callParent();
    },
});

Ext.define('PBS.panel.MatchFieldSettings', {
    extend: 'Ext.panel.Panel',
    xtype: 'pbsNotificationMatchFieldSettings',
    border: false,
    layout: 'anchor',
    // Hide initially to avoid glitches when opening the window
    hidden: true,
    bind: {
	hidden: '{!typeIsMatchField}',
    },
    controller: {
	xclass: 'Ext.app.ViewController',

	control: {
	    'field[reference=fieldSelector]': {
		change: function(field) {
		    let view = this.getView();
		    let valueField = view.down('field[reference=valueSelector]');
		    let store = valueField.getStore();
		    let val = field.getValue();

		    if (val) {
			store.setFilters([
			    {
				property: 'field',
				value: val,
			    },
			]);
		    }
		},
	    },
	},
    },
    viewModel: {
	// parent is set in `initComponents`
	formulas: {
	    typeIsMatchField: {
		bind: {
		    bindTo: '{selectedRecord}',
		    deep: true,
		},
		get: function(record) {
		    return record?.get('type') === 'match-field';
		},
	    },
	    isRegex: function(get) {
		return get('matchFieldType') === 'regex';
	    },
	    matchFieldType: {
		bind: {
		    bindTo: '{selectedRecord}',
		    deep: true,
		},
		set: function(value) {
		    let record = this.get('selectedRecord');
		    let currentData = record.get('data');

		    let newValue = [];

		    // Build equivalent regular expression if switching
		    // to 'regex' mode
		    if (value === 'regex') {
			let regexVal = "^";
			if (currentData.value) {
			    regexVal += `(${currentData.value.join('|')})`;
			}
			regexVal += "$";
			newValue.push(regexVal);
		    }

		    record.set({
			data: {
			    ...currentData,
			    type: value,
			    value: newValue,
			},
		    });
		},
		get: function(record) {
		    return record?.get('data')?.type;
		},
	    },
	    matchFieldField: {
		bind: {
		    bindTo: '{selectedRecord}',
		    deep: true,
		},
		set: function(value) {
		    let record = this.get('selectedRecord');
		    let currentData = record.get('data');

		    record.set({
			data: {
			    ...currentData,
			    field: value,
			    // Reset value if field changes
			    value: [],
			},
		    });
		},
		get: function(record) {
		    return record?.get('data')?.field;
		},
	    },
	    matchFieldValue: {
		bind: {
		    bindTo: '{selectedRecord}',
		    deep: true,
		},
		set: function(value) {
		    let record = this.get('selectedRecord');
		    let currentData = record.get('data');
		    record.set({
			data: {
			    ...currentData,
			    value: value,
			},
		    });
		},
		get: function(record) {
		    return record?.get('data')?.value;
		},
	    },
	},
    },

    initComponent: function() {
	let me = this;

	let store = Ext.create('Ext.data.Store', {
	    model: 'pbs-notification-fields',
	    autoLoad: true,
	    proxy: {
		type: 'proxmox',
		url: `/api2/json/${me.baseUrl}/matcher-fields`,
	    },
	    listeners: {
		'load': function() {
		    this.each(function(record) {
			record.set({
			    description:
				Proxmox.Utils.formatNotificationFieldName(
				    record.get('name'),
				),
			});
		    });

		    // Commit changes so that the description field is not marked
		    // as dirty
		    this.commitChanges();
		},
	    },
	});

	let valueStore = Ext.create('Ext.data.Store', {
	    model: 'pbs-notification-field-values',
	    autoLoad: true,
	    proxy: {
		type: 'proxmox',

		url: `/api2/json/${me.baseUrl}/matcher-field-values`,
	    },
	    listeners: {
		'load': function() {
		    this.each(function(record) {
			if (record.get('field') === 'type') {
			    record.set({
				comment:
				    Proxmox.Utils.formatNotificationFieldValue(
					record.get('value'),
				    ),
			    });
			}
		    }, this, true);

		    // Commit changes so that the description field is not marked
		    // as dirty
		    this.commitChanges();
		},
	    },
	});

	Ext.apply(me.viewModel, {
	    parent: me.up('pmxNotificationMatchRulesEditPanel').getViewModel(),
	});
	Ext.apply(me, {
	    items: [
		{
		    fieldLabel: gettext('Match Type'),
		    xtype: 'proxmoxKVComboBox',
		    reference: 'type',
		    isFormField: false,
		    allowBlank: false,
		    submitValue: false,
		    field: 'type',

		    bind: {
			value: '{matchFieldType}',
		    },

		    comboItems: [
			['exact', gettext('Exact')],
			['regex', gettext('Regex')],
		    ],
		},
		{
		    fieldLabel: gettext('Field'),
		    reference: 'fieldSelector',
		    xtype: 'proxmoxComboGrid',
		    isFormField: false,
		    submitValue: false,
		    allowBlank: false,
		    editable: false,
		    store: store,
		    queryMode: 'local',
		    valueField: 'name',
		    displayField: 'description',
		    field: 'field',
		    bind: {
			value: '{matchFieldField}',
		    },
		    listConfig: {
			columns: [
			    {
				header: gettext('Description'),
				dataIndex: 'description',
				flex: 2,
			    },
			    {
				header: gettext('Field Name'),
				dataIndex: 'name',
				flex: 1,
			    },
			],
		    },
		},
		{
		    fieldLabel: gettext('Value'),
		    reference: 'valueSelector',
		    xtype: 'proxmoxComboGrid',
		    autoSelect: false,
		    editable: false,
		    isFormField: false,
		    submitValue: false,
		    allowBlank: false,
		    showClearTrigger: true,
		    field: 'value',
		    store: valueStore,
		    valueField: 'value',
		    displayField: 'value',
		    notFoundIsValid: false,
		    multiSelect: true,
		    bind: {
			value: '{matchFieldValue}',
			hidden: '{isRegex}',
		    },
		    listConfig: {
			columns: [
			    {
				header: gettext('Value'),
				dataIndex: 'value',
				flex: 1,
			    },
			    {
				header: gettext('Comment'),
				dataIndex: 'comment',
				flex: 2,
			    },
			],
		    },
		},
		{
		    fieldLabel: gettext('Regex'),
		    xtype: 'proxmoxtextfield',
		    editable: true,
		    isFormField: false,
		    submitValue: false,
		    allowBlank: false,
		    field: 'value',
		    bind: {
			value: '{matchFieldValue}',
			hidden: '{!isRegex}',
		    },
		},
	    ],
	});
	me.callParent();
    },
});
